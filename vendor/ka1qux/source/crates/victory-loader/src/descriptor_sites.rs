//! Renderer port #2 (`VICTORY_PORT_DESCRIPTORS=1`, read in `victory-gpu`'s
//! `port_descriptors`): the title's own code that copies descriptor records
//! into its shader-visible heap as bytes tells the layer which slots it wrote.
//!
//! What the title does: each draw's and each dispatch's descriptor tables are
//! allocated from its frame allocator (whose memory is the placed
//! shader-visible view heap) and filled by a few functions, one record at a
//! time: the slot's byte offset in the table is masked, a 32-byte record is
//! loaded into a vector register and `vmovups [table + offset], ymm0` stores
//! it. The stores can be found from the write faults the layer sees (the book
//! records "the instruction at a made the fault"), and a search of all the code
//! for the same instruction shape finds those and no other.
//!
//! Nothing about the title is built in: *where* the stores are is told by the
//! user's profile (`store` lines of [`crate::title_profile`]: byte patterns
//! that must each match exactly once); here only the shape of the instruction
//! is known.
//!
//! Each store becomes a jump to code of ours that does the store, then, if
//! the address lies in the heap `victory-gpu` names in [`Marks`], sets the
//! slot's byte in a map of slots and its 4 KiB chunk's byte in a map of
//! chunks, and jumps back. The flags are clobbered: the profile's patterns
//! run on to the instructions after the store that set them before any use.
//! Every register is put back.
//!
//! A store that is not the expected form of instruction is left alone and said
//! so; so is a pattern of the profile that matches nowhere or twice (another
//! build of the title).

use crate::title_profile::{Code, Profile, find_once, note};

/// The cells the stubs read, `#[repr(C)]` in `victory-gpu`: the heap's first
/// byte, its length in bytes (0: none, nothing is marked), the map of slots
/// (a byte each), the map of chunks (a byte for 128 slots).
pub const MARKS_LEN: usize = 32;

/// A store of the title's to hook: its offset in the image and its length
/// in bytes (5 to 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Store {
    pub at: usize,
    pub len: usize,
}

/// Each store of the profile found once in `code`, and a note for each that
/// was not.
pub fn locate(profile: &Profile, code: &Code) -> (Vec<Store>, Vec<String>) {
    let (mut stores, mut notes) = (Vec::new(), Vec::new());
    for (n, place) in profile.stores.iter().enumerate() {
        match find_once(code, &place.pattern) {
            Ok(found) => stores.push(Store { at: found + place.at, len: place.len }),
            Err(why) => notes.push(note("store", n, &why)),
        }
    }
    (stores, notes)
}

/// `lea rax, [base+index]` for a `vmovups [base+index], ymm0` (VEX, ModRM
/// 04h with a SIB byte, no displacement, the stack pointer not its base):
/// `None` for another form.
fn lea_of(store: &[u8]) -> Option<[u8; 4]> {
    let (x, b, op_at) = match *store.first()? {
        0xc5 => (false, false, 2),
        0xc4 => {
            let p = *store.get(1)?;
            if p & 0x1f != 1 {
                return None;
            }
            (p & 0x40 == 0, p & 0x20 == 0, 3)
        }
        _ => return None,
    };
    let (op, modrm, sib) = (*store.get(op_at)?, *store.get(op_at + 1)?, *store.get(op_at + 2)?);
    if op != 0x11 || modrm != 0x04 || store.len() != op_at + 3 {
        return None;
    }
    // No base (mode 0, base 101), or the stack pointer as base, or rsp as index (none).
    if sib & 7 == 5 || (sib & 7 == 4 && !b) || (sib >> 3 & 7 == 4 && !x) {
        return None;
    }
    Some([0x48 | u8::from(x) << 1 | u8::from(b), 0x8d, 0x04, sib])
}

/// The code at `to` for the store `store` at `at`: the store, the marks, back
/// to `at + store.len()`.
fn stub(store: &[u8], at: usize, to: usize, marks: usize) -> Option<Vec<u8>> {
    let lea = lea_of(store)?;
    let mut out = store.to_vec();
    out.push(0x50); // push rax
    out.extend(lea); // lea rax, [base+index]
    out.extend([0x52, 0x51]); // push rdx ; push rcx
    out.extend([0x48, 0xba]); // mov rdx, marks
    out.extend((marks as u64).to_le_bytes());
    out.extend([0x48, 0x2b, 0x02]); // sub rax, [rdx]
    out.extend([0x48, 0x3b, 0x42, 0x08]); // cmp rax, [rdx+8]
    out.extend([0x73, 0x18]); // jae past the marks
    out.extend([0x48, 0xc1, 0xe8, 0x05]); // shr rax, 5: the slot
    out.extend([0x48, 0x8b, 0x4a, 0x10]); // mov rcx, [rdx+16]
    out.extend([0xc6, 0x04, 0x01, 0x01]); // mov byte [rcx+rax], 1
    out.extend([0x48, 0xc1, 0xe8, 0x07]); // shr rax, 7: the chunk
    out.extend([0x48, 0x8b, 0x4a, 0x18]); // mov rcx, [rdx+24]
    out.extend([0xc6, 0x04, 0x01, 0x01]); // mov byte [rcx+rax], 1
    out.extend([0x59, 0x5a, 0x58]); // pop rcx ; pop rdx ; pop rax
    let back = crate::stubs::jump(to + out.len(), at + store.len())?;
    out.extend(back);
    Some(out)
}

/// What became of a store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hooked {
    Done,
    /// Outside the image, not the expected form, no room for code near it, or
    /// Windows refused.
    Failed(&'static str),
}

/// Hooks `stores` in the image of `size` bytes at `base`; the stubs read the
/// cells at `marks` ([`MARKS_LEN`] bytes). What became of each, said in the
/// log too.
///
/// # Safety
/// The image stays mapped and no thread runs its code yet; `marks` stays
/// valid for as long as the image runs.
pub unsafe fn apply(base: usize, size: usize, marks: usize, stores: &[Store]) -> Vec<Hooked> {
    use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
    use windows_sys::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut room: Option<(usize, usize)> = None;
    let mut out = Vec::new();
    for site in stores {
        let done = (|| {
            if !(5..=16).contains(&site.len) {
                return Hooked::Failed("shorter than a jump, or not an instruction's length");
            }
            if site.at.checked_add(site.len).is_none_or(|end| end > size) {
                return Hooked::Failed("outside the image");
            }
            let at = base + site.at;
            // SAFETY: inside the mapped image (checked above).
            let store = unsafe { std::slice::from_raw_parts(at as *const u8, site.len) }.to_vec();
            if lea_of(&store).is_none() {
                return Hooked::Failed("not a store of the expected form");
            }
            if room.is_none() {
                room = crate::stubs::near(at).map(|place| (place, 0));
            }
            let Some((place, used)) = room.as_mut() else {
                return Hooked::Failed("no room for code near the title's");
            };
            let to = *place + *used;
            let (Some(body), Some(jump)) = (stub(&store, at, to, marks), crate::stubs::jump(at, to)) else {
                return Hooked::Failed("out of reach of the code made for it");
            };
            if *used + body.len() > crate::stubs::STUBS_LEN {
                return Hooked::Failed("no room for code near the title's");
            }
            let mut old = 0;
            // SAFETY: our own code memory, which nothing runs yet; then the
            // store, whose page is made writable for it and put back, and
            // which nothing runs either.
            unsafe {
                std::ptr::copy_nonoverlapping(body.as_ptr(), to as *mut u8, body.len());
                if VirtualProtect(at as *const _, site.len, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
                    return Hooked::Failed("Windows would not make it writable");
                }
                std::ptr::write_bytes(at as *mut u8, 0x90, site.len);
                std::ptr::copy_nonoverlapping(jump.as_ptr(), at as *mut u8, jump.len());
                VirtualProtect(at as *const _, site.len, old, &mut old);
                FlushInstructionCache(GetCurrentProcess(), at as *const _, site.len);
            }
            *used += body.len().next_multiple_of(16);
            Hooked::Done
        })();
        let offset = format_args!("{:#x}", site.at);
        match done {
            Hooked::Done => tracing::info!(site = %offset, "renderer port 2: a descriptor record store of the title patched to tell the layer its slot (VICTORY_PORT_DESCRIPTORS)"),
            Hooked::Failed(why) => tracing::warn!(site = %offset, why, "renderer port 2: not patched"),
        }
        out.push(done);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made-up registers for the stores: (base, index), numbered as the CPU
    /// does (rcx 1, rsi 6, rdi 7, r8 8).
    const PAIRS: [(u8, u8); 6] = [(1, 6), (9, 6), (9, 8), (7, 8), (10, 8), (10, 9)];

    /// `vmovups [base+index], ymm0`, in the shortest VEX form the registers need.
    fn store(base: u8, index: u8) -> Vec<u8> {
        let sib = (index & 7) << 3 | (base & 7);
        if base < 8 && index < 8 {
            vec![0xc5, 0xfc, 0x11, 0x04, sib]
        } else {
            vec![0xc4, 0x80 | u8::from(index < 8) << 6 | u8::from(base < 8) << 5 | 1, 0x7c, 0x11, 0x04, sib]
        }
    }

    #[test]
    fn a_store_lea_s_its_address() {
        for (base, index) in PAIRS {
            let lea = lea_of(&store(base, index)).unwrap();
            assert_eq!(lea, [0x48 | (index >> 3) << 1 | (base >> 3), 0x8d, 0x04, (index & 7) << 3 | (base & 7)]);
        }
        // A store with a displacement, of the stack, of another kind: not taken.
        assert!(lea_of(&[0xc5, 0xfc, 0x11, 0x44, 0x31, 0x08]).is_none());
        assert!(lea_of(&[0xc5, 0xfc, 0x11, 0x04, 0x24]).is_none());
        assert!(lea_of(&[0xc5, 0xfc, 0x10, 0x04, 0x31]).is_none());
        assert!(lea_of(&[0xc5, 0xfc, 0x11, 0x04, 0x31, 0x90]).is_none(), "longer than the instruction");
    }

    /// The marks `victory-gpu` keeps, for a test.
    #[repr(C)]
    struct Marks {
        start: u64,
        len: u64,
        slots: *mut u8,
        chunks: *mut u8,
    }

    /// `mov reg, imm64`.
    fn mov_imm(reg: u8, value: u64) -> Vec<u8> {
        let mut code = vec![0x48 | reg >> 3, 0xb8 | (reg & 7)];
        code.extend(value.to_le_bytes());
        code
    }

    /// A function of code of a test's that loads a record, runs the store,
    /// sets the flags, and writes rax, rcx, rdx and the store's two registers
    /// to `out`; and where the store is in it. rcx and rdx hold made-up values
    /// the store must leave be.
    fn function(pair: (u8, u8), table: u64, offset: u64, record: u64, out: u64) -> (Vec<u8>, usize) {
        let (base, index) = pair;
        let mut code: Vec<u8> = vec![0x56, 0x57]; // push rsi ; push rdi
        code.extend(mov_imm(1, 0x1111_1111_1111)); // rcx
        code.extend(mov_imm(2, 0x2222_2222_2222)); // rdx
        code.extend(mov_imm(0, record));
        code.extend([0xc5, 0xfc, 0x10, 0x00]); // vmovups ymm0, [rax]
        code.extend(mov_imm(base, table));
        code.extend(mov_imm(index, offset));
        let at = code.len();
        code.extend(store(base, index));
        code.extend([0x4d, 0x39, 0xdb]); // cmp r11, r11: the flags are set after the store
        code.extend(mov_imm(11, out));
        for (n, reg) in [0u8, 1, 2, base, index].into_iter().enumerate() {
            // mov [r11 + 8 * n], reg
            code.extend([0x49 | (reg >> 3) << 2, 0x89, 0x43 | (reg & 7) << 3, 8 * n as u8]);
        }
        code.extend([0x5f, 0x5e, 0xc5, 0xf8, 0x77, 0xc3]); // pop rdi ; pop rsi ; vzeroupper ; ret
        (code, at)
    }

    /// Each form of store in code of a test's, twice, one copy hooked, both
    /// run: the record is stored as before, the registers the store leaves are
    /// the same, and the slot and its chunk are marked when the table is in the
    /// heap, not otherwise.
    #[test]
    fn a_hooked_store_stores_and_marks_its_slot() {
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};
        type F = unsafe extern "system" fn();
        for pair in PAIRS {
            // Slot 0x40 + 3 of the heap's start; then before the heap, which marks nothing.
            for marked in [true, false] {
                // SAFETY: fresh memory for code and data, one copy of the code changed before it runs.
                unsafe {
                    let alloc = || VirtualAlloc(std::ptr::null(), 0x10000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) as usize;
                    let (place, heap) = (alloc(), alloc());
                    assert!(place != 0 && heap != 0);
                    let record: [u8; 32] = std::array::from_fn(|k| k as u8 ^ 0x5a);
                    let (mut slots, mut chunks) = (vec![0u8; 0x8000 / 32], vec![0u8; 0x8000 / 32 / 128]);
                    let marks = Box::new(Marks { start: heap as u64 + 0x2000, len: 0x8000, slots: slots.as_mut_ptr(), chunks: chunks.as_mut_ptr() });
                    let table = if marked { heap + 0x2000 + 0x40 * 32 } else { heap + 0x1000 };
                    let mut out = [[0u64; 5]; 2];
                    let (plain, hooked) = (place + 0x100, place + 0x1100);
                    let mut at = 0;
                    for (entry, out) in [plain, hooked].into_iter().zip(&mut out) {
                        let code;
                        (code, at) = function(pair, table as u64, 0x60, record.as_ptr() as u64, out.as_mut_ptr() as u64);
                        std::ptr::copy_nonoverlapping(code.as_ptr(), entry as *mut u8, code.len());
                    }
                    let stores = [Store { at: 0x1100 + at, len: store(pair.0, pair.1).len() }];
                    assert_eq!(apply(place, 0x10000, &raw const *marks as usize, &stores), [Hooked::Done]);
                    assert_eq!(*((hooked + at) as *const u8), 0xe9, "a jump");
                    let target = table + 0x60;
                    for (which, entry) in [plain, hooked].into_iter().enumerate() {
                        std::ptr::write_bytes(target as *mut u8, 0, 32);
                        slots.fill(0);
                        chunks.fill(0);
                        std::mem::transmute::<usize, F>(entry)();
                        assert_eq!(*(target as *const [u8; 32]), record, "{pair:?}: the record is stored");
                        if which == 1 && marked {
                            let slot = 0x40 + 3;
                            assert_eq!(slots[slot], 1);
                            assert_eq!(slots.iter().filter(|&&b| b != 0).count(), 1);
                            assert_eq!(chunks[slot / 128], 1);
                        } else {
                            assert!(slots.iter().chain(&chunks).all(|&b| b == 0), "{pair:?} {which} {marked}");
                        }
                    }
                    assert_eq!(out[0], out[1], "{pair:?}: the registers the store leaves");
                    assert_eq!(out[1][0], record.as_ptr() as u64);
                    assert_eq!(out[1][2], 0x2222_2222_2222);
                    assert_eq!((out[1][3], out[1][4]), (table as u64, 0x60));
                    drop(marks);
                }
            }
        }
    }

    #[test]
    fn what_is_not_a_store_is_left_alone() {
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};
        // SAFETY: fresh memory of our own.
        unsafe {
            let place = VirtualAlloc(std::ptr::null(), 0x10000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) as *mut u8;
            // A store with a displacement, then something else.
            let other = [0xc5, 0xfc, 0x11, 0x44, 0x31, 0x08, 0x90];
            std::ptr::copy_nonoverlapping(other.as_ptr(), place.add(0x100), other.len());
            let results = apply(
                place as usize,
                0x10000,
                0,
                &[
                    Store { at: 0x100, len: 6 },
                    Store { at: 0x100, len: 4 },
                    Store { at: 0xfffd, len: 5 },
                    Store { at: usize::MAX - 2, len: 5 },
                    Store { at: 0x100, len: 17 },
                ],
            );
            assert_eq!(results[0], Hooked::Failed("not a store of the expected form"));
            assert!(matches!(results[1], Hooked::Failed(why) if why.contains("shorter")));
            assert_eq!(results[2], Hooked::Failed("outside the image"));
            assert_eq!(results[3], Hooked::Failed("outside the image"));
            assert!(matches!(results[4], Hooked::Failed(why) if why.contains("shorter")));
            assert_eq!(std::slice::from_raw_parts(place.add(0x100), other.len()), &other[..]);
        }
    }

    #[test]
    fn the_profiles_stores_are_found_once_each() {
        let profile = Profile::parse(
            "store = 4 5 : A1 ?? 33 44 C5 FC 11 04 31 77\nstore = 1 5 : B1 C5 FC 11 04 31 B2\nstore = 0 5 : EE EE EE EE EE\n",
        )
        .unwrap();
        let mut code = vec![0x90u8; 0x20];
        code.extend([0xa1, 0x99, 0x33, 0x44, 0xc5, 0xfc, 0x11, 0x04, 0x31, 0x77]);
        code.extend([0x90; 8]);
        let second = code.len();
        code.extend([0xb1, 0xc5, 0xfc, 0x11, 0x04, 0x31, 0xb2]);
        // The third pattern is here twice: its place is not guessed.
        code.extend([0xee; 6]);
        let (stores, notes) = locate(&profile, &[(0x7000, &code)]);
        assert_eq!(stores, [Store { at: 0x7000 + 0x20 + 4, len: 5 }, Store { at: 0x7000 + second + 1, len: 5 }]);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].starts_with("store 3: not patched") && notes[0].contains("matches 2 places"), "{notes:?}");
    }
}

//! Memory for code of ours near the title's code, and the jump that reaches it: what the places
//! changed at load ([`crate::descriptor_sites`], [`crate::write_sites`]) need to put a stub of
//! theirs where the title's instruction was.

/// `jmp rel32` at `at` to `to`, if it reaches.
pub(crate) fn jump(at: usize, to: usize) -> Option<[u8; 5]> {
    let rel = i32::try_from(to as i64 - (at as i64 + 5)).ok()?;
    let [a, b, c, d] = rel.to_le_bytes();
    Some([0xe9, a, b, c, d])
}

/// The size of one block of stubs.
pub(crate) const STUBS_LEN: usize = 64 << 10;

/// Memory for code within reach of a `jmp rel32` at `at`: a block of [`STUBS_LEN`] bytes, readable,
/// writable and executable, at the nearest free place (1 MiB steps, both sides, up to 1.75 GiB).
pub(crate) fn near(at: usize) -> Option<usize> {
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc};
    let at = at & !0xffff;
    let mut places = (0x10_0000..0x7000_0000).step_by(0x10_0000).flat_map(|d| [at.checked_sub(d), at.checked_add(d)]).flatten();
    places.find_map(|place| {
        // SAFETY: reserves free memory at that address or fails.
        let got = unsafe { VirtualAlloc(place as *const _, STUBS_LEN, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
        (!got.is_null()).then_some(got as usize)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_jump_reaches_both_ways_and_not_past_two_gigabytes() {
        assert_eq!(jump(0x1000, 0x1005), Some([0xe9, 0, 0, 0, 0]));
        assert_eq!(jump(0x1000, 0x1000), Some([0xe9, 0xfb, 0xff, 0xff, 0xff]));
        assert!(jump(0x2000_0000, 0x2000_0000 + 5 + i32::MAX as usize).is_some());
        assert!(jump(0x2000_0000, 0x2000_0000 + 6 + i32::MAX as usize).is_none());
        assert!(jump(0x1000, 0x9000_0000_0000).is_none(), "out of reach");
        assert!(jump(0x9000_0000_0000, 0x1000).is_none(), "out of reach backwards");
    }

    #[test]
    fn a_block_comes_within_reach_of_its_place() {
        // Near this very code.
        let at = a_block_comes_within_reach_of_its_place as *const () as usize;
        let place = near(at).expect("a free block within reach");
        assert!(jump(at, place).is_some() && jump(place, at).is_some());
        // SAFETY: the block was just made, readable and writable, and nothing else holds it.
        unsafe {
            (place as *mut u8).write(0xc3);
            assert_eq!((place as *const u8).read(), 0xc3);
            windows_sys::Win32::System::Memory::VirtualFree(place as *mut _, 0, windows_sys::Win32::System::Memory::MEM_RELEASE);
        }
    }
}

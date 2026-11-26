//! VICTORY_CULL_PASSALL=1 (diagnostics): the meshlet-cull shaders take every
//! meshlet as visible, by turning their last visibility test into s_nop.

// shaders that get it: their translations skip the cache, both ways
const HASHES: [&str; 4] = ["3b471c122be79b3e", "fc5e66f8f873d3bb", "9f017d9011377efa", "0452fc2d2e2fc4c3"];

// the test's byte offset, its word, and the word after it
const TESTS: [(usize, u32, u32); 2] = [
    // v_cmpx_eq_u32 exec, -1, v15, then s_cbranch_execz (3b47, fc5e)
    (0x2930, 0x7da4_1ec1, 0xbf88_0000),
    // s_and_b64 s[0:1], s[0:1], vcc, then s_and_saveexec_b64 (9f01)
    (0x110c, 0x8780_6a00, 0xbe80_0000),
];
// 0452's test sits elsewhere
const TESTS_0452: (usize, u32, u32) = (0x1170, 0x8780_6a00, 0xbe80_0000);

const S_NOP: u32 = 0xbf80_0000;

pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var("VICTORY_CULL_PASSALL").is_ok_and(|v| v == "1");
        if on {
            victory_shader::set_code_patch(patch);
        }
        on
    })
}

/// Whether this shader's translation must skip the cache.
pub(crate) fn wanted(container: &[u8]) -> bool {
    on() && HASHES.contains(&&blake3::hash(container).to_hex()[..16])
}

fn patch(code: &mut Vec<u8>) {
    for (pc, word, next) in TESTS.into_iter().chain([TESTS_0452]) {
        if nop_at(code, pc, word, next) {
            tracing::warn!(len = code.len(), pc, "VICTORY_CULL_PASSALL visibility test made s_nop");
        }
    }
}

// the word at pc becomes s_nop when it and the next word's opcode match
fn nop_at(code: &mut [u8], pc: usize, word: u32, next: u32) -> bool {
    let at = |i: usize| code.get(i..i + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    if at(pc) != Some(word) || at(pc + 4).map(|w| w & 0xffff_0000) != Some(next) {
        return false;
    }
    code[pc..pc + 4].copy_from_slice(&S_NOP.to_le_bytes());
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_matching_test_becomes_a_no_op() {
        let mut code = vec![0u8; 0x2940];
        code[0x2930..0x2934].copy_from_slice(&0x7da4_1ec1u32.to_le_bytes());
        code[0x2934..0x2938].copy_from_slice(&0xbf88_0012u32.to_le_bytes());
        let before = code.clone();
        assert!(super::nop_at(&mut code, 0x2930, 0x7da4_1ec1, 0xbf88_0000));
        assert_eq!(&code[0x2930..0x2934], &super::S_NOP.to_le_bytes());
        assert_eq!(code[..0x2930], before[..0x2930]);
        assert_eq!(code[0x2934..], before[0x2934..]);
        assert!(!super::nop_at(&mut code, 0x2930, 0x7da4_1ec1, 0xbf88_0000), "already patched");
        assert!(!super::nop_at(&mut code, 0x110c, 0x8780_6a00, 0xbe80_0000), "not there");
    }
}

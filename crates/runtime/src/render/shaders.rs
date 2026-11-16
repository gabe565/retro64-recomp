//! Shader code generated at build time from `shaders/*.wgsl`.
include!(concat!(env!("OUT_DIR"), "/shaders.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    /// The Direct3D 12 path: HLSL generated from the SPIR-V must use SDL3's D3D12
    /// register layout (fragment t0/s0 in space2, b0 in space3) and TEXCOORD
    /// semantics, or SDL's root signature will not match.
    #[test]
    fn hlsl_uses_sdl_d3d12_registers_and_semantics() {
        for r in ["register(t0, space2)", "register(s0, space2)", "register(b0, space3)"] {
            assert!(FIXED_FS_MAIN_HLSL.contains(r), "fragment HLSL lacks {r}");
        }
        assert!(!FIXED_VS_MAIN_HLSL.contains("register("), "vertex shader has no resources");
        for i in 0..4 {
            assert!(FIXED_VS_MAIN_HLSL.contains(&format!(": TEXCOORD{i};")), "vertex input TEXCOORD{i}");
        }
        for i in 0..3 {
            assert!(FIXED_FS_MAIN_HLSL.contains(&format!(": TEXCOORD{i};")), "fragment input TEXCOORD{i}");
        }
        assert!(FIXED_VS_MAIN_HLSL.contains(" main(") && FIXED_FS_MAIN_HLSL.contains(" main("));
    }

    /// When DXIL was built, it is a signed DXBC container (DXC's validator fills
    /// in the hash; Direct3D 12 rejects unsigned shaders).
    #[test]
    fn dxil_when_built_is_signed() {
        for code in [FIXED_VS_MAIN_DXIL, FIXED_FS_MAIN_DXIL] {
            if code.is_empty() {
                continue;
            }
            assert_eq!(&code[..4], b"DXBC");
            assert_ne!(&code[4..20], &[0u8; 16], "unsigned DXIL");
        }
    }
}

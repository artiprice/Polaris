use polaris_ext::{mcp as _, skills};

#[test]
fn facade_reexports_both_extension_mechanisms() {
    let _skill_surface = core::any::type_name::<skills::SkillRegistry>();
    assert_eq!(skills::MAX_SKILLS, 2_000);
}

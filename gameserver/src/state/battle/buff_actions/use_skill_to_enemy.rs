//! buff_act 759 UseSkillToEnemy — round-end follow-up skill
//! (Semmelweis Truth Revealed family).

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 759;

/// Returns `(skill_id, param)` or `None` if the buff isn't carrying
/// this feature or `skill_id` is 0.
pub fn buff_get_use_skill_to_enemy_params(buff_id: i32) -> Option<(i32, i32)> {
    let parts = super::find_feature_parts(buff_id, "UseSkillToEnemy")?;
    let skill_id = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let param = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
    if skill_id > 0 {
        Some((skill_id, param))
    } else {
        None
    }
}

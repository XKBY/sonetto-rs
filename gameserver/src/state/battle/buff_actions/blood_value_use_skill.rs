//! buff_act 1009 BloodValueUseSkill — fires a follow-up skill when
//! bloodtithe value crosses a configured threshold.

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 1009;

/// Returns `(prerequisite_buff_id, blood_value_threshold, wrapper_skill_id)`
/// or `None` if the buff isn't carrying this feature or
/// `wrapper_skill_id` is 0.
pub fn buff_get_blood_value_use_skill_params(buff_id: i32) -> Option<(i32, i32, i32)> {
    let parts = super::find_feature_parts(buff_id, "BloodValueUseSkill")?;
    let prerequisite_buff_id = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let blood_value_threshold = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
    let wrapper_skill_id = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
    if wrapper_skill_id > 0 {
        Some((
            prerequisite_buff_id,
            blood_value_threshold,
            wrapper_skill_id,
        ))
    } else {
        None
    }
}

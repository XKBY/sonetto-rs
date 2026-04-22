//! Handler for buff_act 1009 BloodValueUseSkill — fires a follow-up skill when bloodtithe value crosses a configured threshold.

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 1009;

pub fn buff_get_blood_value_use_skill_params(buff_id: i32) -> Option<(i32, i32, i32)> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    for segment in buff.features.split('|') {
        let parts: Vec<&str> = segment.split('#').collect();
        let feature_id: i32 = parts.first()?.trim().parse().ok()?;
        let Some(act_type) = cfg
            .buff_act
            .iter()
            .find(|a| a.id == feature_id)
            .map(|a| a.r#type.as_str())
        else {
            continue;
        };
        if act_type != "BloodValueUseSkill" {
            continue;
        }
        let prerequisite_buff_id = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let blood_value_threshold = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
        let wrapper_skill_id = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        if wrapper_skill_id > 0 {
            return Some((
                prerequisite_buff_id,
                blood_value_threshold,
                wrapper_skill_id,
            ));
        }
    }
    None
}

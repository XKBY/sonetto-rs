//! Handler for buff_act 759 UseSkillToEnemy — round-end follow-up skill (Semmelweis Truth Revealed family).

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 759;

pub fn buff_get_use_skill_to_enemy_params(buff_id: i32) -> Option<(i32, i32)> {
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
        if act_type != "UseSkillToEnemy" {
            continue;
        }
        let skill_id = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let param = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
        if skill_id > 0 {
            return Some((skill_id, param));
        }
    }
    None
}

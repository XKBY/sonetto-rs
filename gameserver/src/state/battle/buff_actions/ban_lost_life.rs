//! Handler for buff_act 1008 BanLostLife — clamps LostLife damage to a configured floor.

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 1008;

pub fn buff_get_ban_lost_life_floor(buff_id: i32) -> Option<i32> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    buff.features.split('|').find_map(|entry| {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id: i32 = parts.first()?.trim().parse().ok()?;
        let is_ban = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == "BanLostLife")
            .unwrap_or(false);
        if is_ban {
            parts.get(1)?.trim().parse().ok()
        } else {
            None
        }
    })
}

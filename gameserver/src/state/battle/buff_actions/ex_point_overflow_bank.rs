//! Handler for buff_act 806 ExPointOverflowBank — queries EX-cap overflow behavior for buff-driven EX gains.

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 806;

pub fn buff_get_ex_point_overflow(buff_id: i32) -> Option<i32> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    buff.features.split('|').find_map(|entry| {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id: i32 = parts.first()?.trim().parse().ok()?;
        let is_overflow = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == "ExPointOverflowBank")
            .unwrap_or(false);
        if is_overflow {
            parts.get(1)?.trim().parse().ok()
        } else {
            None
        }
    })
}

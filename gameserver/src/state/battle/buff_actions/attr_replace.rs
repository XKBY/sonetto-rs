//! Handlers for buff_act 1005/1007 AttrOnlyCalDamageReplaceAttr family — swaps source attribute in damage-calc (NuoDiKa uses this to scale off Max HP instead of ATK).

#[allow(dead_code)]
pub const BUFF_ACT_ID_AD_CREATOR: i32 = 1005;
#[allow(dead_code)]
pub const BUFF_ACT_ID_REPLACE: i32 = 1007;

pub fn buff_get_attr_replace_permille(buff_id: i32) -> Option<i32> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    buff.features.split('|').find_map(|entry| {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id: i32 = parts.first()?.trim().parse().ok()?;
        let is_replace = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == "AttrOnlyCalDamageReplaceAttr")
            .unwrap_or(false);
        if !is_replace {
            return None;
        }
        parts.get(3)?.trim().parse().ok()
    })
}

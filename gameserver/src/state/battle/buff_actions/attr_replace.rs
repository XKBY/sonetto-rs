//! buff_act 1005/1007 AttrOnlyCalDamageReplaceAttr — swaps source
//! attribute in damage-calc (Nuodika scales off Max HP instead of ATK).

#[allow(dead_code)]
pub const BUFF_ACT_ID_AD_CREATOR: i32 = 1005;
#[allow(dead_code)]
pub const BUFF_ACT_ID_REPLACE: i32 = 1007;

pub fn buff_get_attr_replace_permille(buff_id: i32) -> Option<i32> {
    super::feature_param_i32(buff_id, "AttrOnlyCalDamageReplaceAttr", 3)
}

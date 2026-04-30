//! buff_act 1008 BanLostLife — clamps LostLife damage to a floor.

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 1008;

pub fn buff_get_ban_lost_life_floor(buff_id: i32) -> Option<i32> {
    super::feature_param_i32(buff_id, "BanLostLife", 1)
}

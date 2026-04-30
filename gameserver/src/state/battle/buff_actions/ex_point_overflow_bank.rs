//! buff_act 806 ExPointOverflowBank — EX-cap overflow cap value.

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 806;

pub fn buff_get_ex_point_overflow(buff_id: i32) -> Option<i32> {
    super::feature_param_i32(buff_id, "ExPointOverflowBank", 1)
}

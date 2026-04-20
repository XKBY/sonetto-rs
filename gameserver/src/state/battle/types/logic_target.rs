/// Targets only the caster itself.
pub const ME: &[i32] = &[103];

/// Targets all allies (my side).
pub const MY_SIDE_ALL: &[i32] = &[101, 104, 105];

/// Targets a single ally.
pub const SINGLE_ALLY: &[i32] = &[1, 106, 107, 108, 204, 205, 207, 303];

/// Targets a single ally or random ally.
pub const SINGLE_OR_RANDOM_ALLY: &[i32] = &[201, 206];

/// Targets all enemies.
pub const ENEMY_SIDE_ALL: &[i32] = &[202, 301, 302];

/// Targets a single enemy by position index.
pub const ENEMY_SIDE_INDEX: &[i32] = &[226, 227, 228, 229];

/// Targets the enemy with the most HP.
pub const ENEMY_MOST_HP: &[i32] = &[208];

/// Special target — context-dependent (e.g. no target, global).
pub const SPECIAL: &[i32] = &[0];

/// Secondary target (set by previous skill step).
pub const SECONDARY_TARGET: &[i32] = &[216];

/// Position-based targets.
pub mod position {
    pub const POS_1: i32 = 222;
    pub const POS_2: i32 = 223;
    pub const POS_3: i32 = 224;
    pub const POS_4: i32 = 225;
}

pub fn is_me(target: i32) -> bool {
    ME.contains(&target)
}

pub fn is_my_side_all(target: i32) -> bool {
    MY_SIDE_ALL.contains(&target)
}

pub fn is_enemy_side_all(target: i32) -> bool {
    ENEMY_SIDE_ALL.contains(&target)
}

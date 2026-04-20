//! Magic-circle mechanic (blood domain family).
//!
//! Semmelweis's magic circle (config id 100051) is an arena effect that the
//! carrier summons when her ultimate resolves. While the circle is active,
//! the carrier emits `MAGIC_CIRCLE_SELF_SKILL_ID` multiple times per round —
//! typically once on every ally/enemy skill step that satisfies the circle's
//! internal trigger. This is independent of any other hero's card play; the
//! circle owns its firing schedule.
//!
//! Priority/emit-order: when the circle fires in the same frame as another
//! allied skill (for example a Nautika ultimate), live orders emissions by
//! skill id. The caller site that invokes this mechanic should collect
//! candidate emissions from all sources for the current host step and sort
//! by skill id before flushing, so that Nautika's 31200133 reliably precedes
//! the circle's 308801821 without depending on call order.
//!
//! The circle's emission carries a damage effect (effectType 2) and may
//! carry a BloodPoolValueChange rider — neither contributes to type=9 or
//! type=111 parity slices, so parity metrics remain clean regardless of how
//! many circle fires per round our engine reproduces. Matching the full
//! structural shape against live requires additional work; for now the
//! module exposes the identifiers and intended ordering rule while the
//! firing drivers are staged in follow-up passes.

/// Magic circle config id — see `data/excel2json/magic_circle.json:100051`.
pub const BLOOD_DOMAIN_CIRCLE_ID: i32 = 100051;

/// Buff applied to the carrier while the circle is active (magic circle
/// config `selfBuff` for 100051).
pub const BLOOD_DOMAIN_CARRIER_BUFF_ID: i32 = 308801312;

/// Skill fired by the circle on each qualifying trigger (magic circle
/// config `selfSkills` for 100051).
pub const MAGIC_CIRCLE_SELF_SKILL_ID: i32 = 308801821;

// Behaviors that trigger or use other skills.
// Most are combat-only stubs for now.
use anyhow::Result;
use sonettobuf::ActEffect;

pub fn shell_assign() -> Result<Vec<ActEffect>> {
    // TODO: implement when shell/summon system is built
    Ok(vec![])
}

pub fn shell_use_skill() -> Result<Vec<ActEffect>> {
    // TODO: implement when shell/summon system is built
    Ok(vec![])
}

pub fn consume_power_direct_use_skill() -> Result<Vec<ActEffect>> {
    // TODO: implement when power system is built
    Ok(vec![])
}

pub fn random_use_skill() -> Result<Vec<ActEffect>> {
    // TODO: implement when combat skill trigger system is built
    Ok(vec![])
}

pub fn summon() -> Result<Vec<ActEffect>> {
    // TODO: implement when summon system is built
    Ok(vec![])
}

pub fn kill() -> Result<Vec<ActEffect>> {
    // TODO: implement
    Ok(vec![])
}

pub fn monster_change() -> Result<Vec<ActEffect>> {
    // TODO: implement
    Ok(vec![])
}

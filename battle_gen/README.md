# battle_gen

## How to add a scenario
1. Create `tests/<scenario_name>/` where `<scenario_name>` is the scenario name (for example `battle2`).
2. Add `live_battle2.json` to that directory as the scenario fight payload.
3. Add one or more `begin_round_N.json` captures (and optional matching `begin_round_N_request.json`) in the same directory.
4. Commit golden snapshots at `tests/golden/<scenario_name>/my_begin_round_N.json`.
5. Run `cargo run -p battle_gen` to generate `tests/runs/<scenario_name>/my_begin_round_N.json` outputs.

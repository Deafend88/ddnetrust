//! PLAN §13.5: the Esc menu's Game buttons against C++. `menus.txt` comes from
//! `rs_parity menus`, which runs a word-for-word copy of `CMenus::RenderGame`'s
//! state-dependent part (menus_ingame.cpp:140-251) on every combination of what it reads
//! (src/tools/rs_parity_menus.cpp).

use ddnet_rs::esc_menu::{GameState, game_buttons};

#[test]
fn game_buttons_match_cpp() {
    let path = format!("{}/../tests/golden/menus.txt", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(path).unwrap();
    let mut cases = 0;
    for line in text.lines() {
        let (state, buttons) = line.split_once('|').unwrap_or((line, ""));
        let v: Vec<i32> = state.split(' ').map(|n| n.parse().unwrap()).collect();
        let s = GameState {
            has_local_id: v[0] != 0,
            local_team: (v[1] != 0).then_some(v[3]),
            game_info: v[2] != 0,
            paused: v[4] != 0,
            spec: v[5] != 0,
            team_play: v[6] != 0,
            received_ddnet_player: v[7] != 0,
            show_ddrace_buttons: v[8] != 0,
            dummy_connecting: v[9] != 0,
        };
        let ours: Vec<&str> = game_buttons(&s).iter().map(|b| b.cpp_label()).collect();
        let cpp: Vec<&str> = buttons.split('|').filter(|b| !b.is_empty()).collect();
        assert_eq!(ours, cpp, "{line}");
        cases += 1;
    }
    assert_eq!(cases, 1536);
}

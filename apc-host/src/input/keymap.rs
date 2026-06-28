use super::types::BTN_LEFT;
use super::types::BTN_MIDDLE;
use super::types::BTN_RIGHT;

pub fn macos_keycode_to_linux(keycode: u16) -> u16 {
    match keycode {
        0 => 30,  // A
        1 => 31,  // S
        2 => 32,  // D
        3 => 33,  // F
        4 => 35,  // H
        5 => 34,  // G
        6 => 44,  // Z
        7 => 45,  // X
        8 => 46,  // C
        9 => 47,  // V
        11 => 48, // B
        12 => 16, // Q
        13 => 17, // W
        14 => 18, // E
        15 => 19, // R
        16 => 21, // Y
        17 => 20, // T

        18 => 2,  // 1
        19 => 3,  // 2
        20 => 4,  // 3
        21 => 5,  // 4
        22 => 7,  // 6
        23 => 6,  // 5
        24 => 13, // EQUAL
        25 => 10, // 9
        26 => 8,  // 7
        27 => 12, // MINUS
        28 => 9,  // 8
        29 => 11, // 0

        30 => 27, // RIGHTBRACE
        31 => 24, // O
        32 => 22, // U
        33 => 26, // LEFTBRACE
        34 => 23, // I
        35 => 25, // P
        36 => 28, // ENTER
        37 => 38, // L
        38 => 36, // J
        39 => 40, // APOSTROPHE
        40 => 37, // K
        41 => 39, // SEMICOLON
        42 => 43, // BACKSLASH
        43 => 51, // COMMA
        44 => 53, // SLASH
        45 => 49, // N
        46 => 50, // M
        47 => 52, // DOT

        48 => 15, // TAB
        49 => 57, // SPACE
        50 => 41, // GRAVE
        51 => 14, // BACKSPACE
        53 => 1,  // ESC

        56 => 42,  // LEFTSHIFT
        57 => 58,  // CAPSLOCK
        58 => 56,  // LEFTALT
        59 => 29,  // LEFTCTRL
        60 => 54,  // RIGHTSHIFT
        61 => 100, // RIGHTALT
        62 => 97,  // RIGHTCTRL

        96 => 63,   // F5
        97 => 64,   // F6
        98 => 65,   // F7
        99 => 61,   // F3
        100 => 66,  // F8
        101 => 67,  // F9
        103 => 87,  // F11
        105 => 210, // F13
        107 => 70,  // F14 → SCROLLLOCK
        109 => 68,  // F10
        111 => 88,  // F12
        113 => 110, // F15 → INSERT
        114 => 102, // HOME
        115 => 102, // HOME
        116 => 104, // PAGEUP
        117 => 111, // DELETE (forward)
        118 => 62,  // F4
        119 => 107, // END
        120 => 60,  // F2
        121 => 109, // PAGEDOWN
        122 => 59,  // F1

        123 => 105, // LEFT
        124 => 106, // RIGHT
        125 => 108, // DOWN
        126 => 103, // UP

        _ => 0,
    }
}

pub fn macos_mouse_button_to_linux(button: u16) -> u16 {
    match button {
        0 => BTN_LEFT,
        1 => BTN_RIGHT,
        2 => BTN_MIDDLE,
        _ => BTN_LEFT,
    }
}

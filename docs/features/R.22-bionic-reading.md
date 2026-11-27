# Ph5: 2/45 | R.22: Bionic reading

**Tag:** R.22; **Priority:** P2; **Phase:** 5; **Needs:** R.16; **Code:** `ecclesia-features::reflow::bionic`
**Plan:** §27 entry `#### Ph5: 2/45 | R.22: Bionic reading`.

## Problem

Readers with ADHD or visual tracking fatigue can lose their place across dense reflowed paragraphs.

## User story

As a reader in reflow mode, I want to toggle script-aware bionic bolding with adjustable prefix strength so my eyes lock onto word beginnings effortlessly in supported scripts.

## UI sketch

- Reflow typography popover includes **Bionic reading** toggle (`BionicMode::Off` / `On { strength }`) and fixation strength slider (`10%`–`80%`, default `40%`).
- Command palette: `read.bionic_toggle`.

## Mode

Read, Study (active inside Reflow mode only).

## Speed and size impact

- Startup: zero cost.
- Budgets touched: O(n) single-pass Unicode scalar segmentation per visible reflow block.
- Installer size: zero new crates.

## Design

- **Model (`ecclesia-features::reflow::bionic`):**
  - `is_bionic_script_char` and `supports_bionic_word` allow Latin, Cyrillic, and Greek words and automatically disable prefix bolding for CJK, Arabic, Hebrew, Thai, and Indic scripts.
  - `transform_text` and `to_html` split supported words into `BionicSegment::Bold` (`<strong>`) and `BionicSegment::Normal`.

## Acceptance tests

- [x] `reflow::bionic::tests::bolds_latin_cyrillic_and_greek_prefixes`: verifies ~40 % prefix bolding on Latin, Cyrillic, and Greek words.
- [x] `reflow::bionic::tests::turns_itself_off_for_cjk_arabic_and_thai`: verifies bionic bolding disables itself automatically for CJK, Arabic, Thai, and Hebrew text.

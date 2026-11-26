# Ph9: 3/13 | G.15: Quick polls

**Tag:** G.15; **Priority:** P2; **Phase:** 9; **Needs:** G.14; **Code:** `excellent-features::classroom::poll`
**Plan:** §27 entry `#### Ph9: 3/13 | G.15`.

## Problem

During live lectures, teachers need instant formative checks from students without switching to external web polling tools or collecting student identities.

## User story

As a teacher, I want to launch a multiple-choice quick poll inside the live classroom session and view a live bar histogram of anonymous responses so that I can gauge comprehension on the spot.

## UI sketch

- Quick Poll modal/drawer inside Live Classroom with question prompt, answer choices, privacy badge ("Anonymous by default"), and NSH `Plot` bar histogram results view.
- Keyboard shortcuts `1`–`9` for students to submit votes cleanly.

## Mode

Present, Teach.

## Speed and size impact

- Startup: loads only inside active classroom sessions.
- Budgets touched: O(N) vote tallying over student tokens.
- Installer size: 0 bytes added.

## Design

- Model: `excellent-features::classroom::poll::{QuickPoll, PollVote, PollTally, PollHistogram, VotePrivacy, PollStatus}`.
- Privacy invariant: `VotePrivacy::Anonymous` is default and strips voter names so `PollHistogram::named_voters` is strictly `None`.

## Acceptance tests

- [ ] `quick_polls_are_anonymous_by_default_and_aggregate_histograms`: Verifies default anonymous mode omits voter identities while accurately tallying votes and percentages.

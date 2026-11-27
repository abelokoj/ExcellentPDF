# Ph9: 2/13 | G.14: Live classroom

**Tag:** G.14; **Priority:** P2; **Phase:** 9; **Needs:** Y.4; **Code:** `ecclesia-features::classroom`
**Plan:** §27 entry `#### Ph9: 2/13 | G.14`.

## Problem

Teachers and presenters in classrooms need every student's PDF viewer to follow page turns and laser focus in real time without forcing students to create accounts.

## User story

As a teacher or presenter, I want to start a live classroom session with a 6-character join code over LAN or a self-hosted relay so that students can join immediately without accounts and follow my reading focus.

## UI sketch

- Classroom toolbar button / Command Palette (`classroom.host`, `classroom.join`) showing the join code, connected student count, mDNS LAN beacon status, and student follow-mode toggle (`LockedToTeacher` / `FreeBrowse`).
- Accessible keyboard navigation and screen-reader status announcements on page sync.

## Mode

Present, Teach, Read.

## Speed and size impact

- Startup: classroom host/client transport initializes only when a session starts or joins.
- Budgets touched: LAN broadcast fan-out completes in < 300 ms for 30+ students.
- Installer size: 0 bytes added (uses std TCP + `ureq` relay).

## Design

- Model: `ecclesia-features::classroom::{ClassroomHost, ClassroomStudent}` over `ClassroomTransport` (`InMemoryClassroomBus`, `TcpClassroomServer`/`TcpClassroomClient`, `RelayHttpTransport`), `JoinCode`, `DiscoveryBeacon`, `ReadingFocus`, `FollowMode`.

## Acceptance tests

- [ ] `thirty_students_follow_teacher_page_changes_well_under_300ms`: 30 students join by code without accounts and receive teacher page/focus updates within 300 ms.
- [ ] `local_tcp_classroom_transport_delivers_focus_updates`: Local TCP loopback transport delivers focus updates between host and student.

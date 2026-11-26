//! Live classroom LAN/relay broadcast host, student follower, and anonymous-by-default
//! quick polls (plan G.14, G.15).

pub mod poll;
pub mod protocol;
pub mod transport;

use std::collections::BTreeMap;
use std::fmt;

use self::poll::{PollHistogram, PollTally, QuickPoll};
use self::protocol::{ClassroomPacket, DiscoveryBeacon, FollowMode, JoinCode, ReadingFocus};
use self::transport::ClassroomTransport;

/// Errors returned by live classroom hosting, joining, and polling.
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub enum ClassroomError {
    /// Join code failed validation or did not match the active session.
    InvalidCode(String),
    /// Wire packet or relay protocol error.
    Protocol(String),
    /// Poll operation error (closed poll, out-of-range option, unknown poll).
    Poll(String),
    /// Underlying socket or I/O error.
    Io(std::io::Error),
}

impl fmt::Display for ClassroomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCode(code) => write!(f, "invalid classroom code: {code}"),
            Self::Protocol(msg) => write!(f, "classroom protocol error: {msg}"),
            Self::Poll(msg) => write!(f, "classroom poll error: {msg}"),
            Self::Io(err) => write!(f, "classroom i/o error: {err}"),
        }
    }
}

impl std::error::Error for ClassroomError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::InvalidCode(_) | Self::Protocol(_) | Self::Poll(_) => None,
        }
    }
}

impl From<std::io::Error> for ClassroomError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Connected student entry tracked by [`ClassroomHost`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedStudent {
    /// Ephemeral student session id.
    pub student_id: String,
    /// Optional display name.
    pub display_name: Option<String>,
}

/// Teacher host session broadcasting reading focus and managing quick polls (plan G.14, G.15).
pub struct ClassroomHost<T: ClassroomTransport> {
    session_title: String,
    join_code: JoinCode,
    transport: T,
    students: BTreeMap<String, ConnectedStudent>,
    current_focus: Option<ReadingFocus>,
    next_seq: u64,
    polls: BTreeMap<String, PollTally>,
}

impl<T: ClassroomTransport> ClassroomHost<T> {
    /// Starts a live classroom session with `session_title` and `join_code` over `transport`.
    #[must_use]
    pub fn new(session_title: &str, join_code: JoinCode, transport: T) -> Self {
        Self {
            session_title: session_title.to_owned(),
            join_code,
            transport,
            students: BTreeMap::new(),
            current_focus: None,
            next_seq: 0,
            polls: BTreeMap::new(),
        }
    }

    /// Join code required by students.
    #[must_use]
    pub const fn join_code(&self) -> &JoinCode {
        &self.join_code
    }

    /// Builds an mDNS [`DiscoveryBeacon`] advertising this classroom on `host_addr`.
    #[must_use]
    pub fn discovery_beacon(&self, host_addr: &str, relay_url: Option<&str>) -> DiscoveryBeacon {
        DiscoveryBeacon {
            session_title: self.session_title.clone(),
            join_code: self.join_code.clone(),
            host_addr: host_addr.to_owned(),
            relay_url: relay_url.map(str::to_owned),
        }
    }

    /// Connected student count.
    #[must_use]
    pub fn student_count(&self) -> usize {
        self.students.len()
    }

    /// Pushes a teacher page/scroll/laser focus update to all connected students.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError`] if transport broadcast fails.
    pub fn broadcast_focus(
        &mut self,
        doc_id: &str,
        page: usize,
        scroll_y: f32,
        zoom: f32,
        laser_point: Option<(f32, f32)>,
    ) -> Result<ReadingFocus, ClassroomError> {
        self.next_seq += 1;
        let focus = ReadingFocus {
            doc_id: doc_id.to_owned(),
            page,
            scroll_y,
            zoom,
            laser_point,
            seq: self.next_seq,
        };
        self.current_focus = Some(focus.clone());
        self.transport
            .send(&ClassroomPacket::FocusUpdate(focus.clone()))?;
        Ok(focus)
    }

    /// Publishes a [`QuickPoll`] to all connected students (plan G.15).
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError`] if transport broadcast fails.
    pub fn publish_poll(&mut self, poll: QuickPoll) -> Result<(), ClassroomError> {
        let id = poll.id.clone();
        self.polls.insert(id, PollTally::new(poll.clone()));
        self.transport.send(&ClassroomPacket::PollPublished(poll))
    }

    /// Closes `poll_id`, broadcasts the final histogram, and returns it.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError::Poll`] if `poll_id` does not exist or transport fails.
    pub fn close_poll(&mut self, poll_id: &str) -> Result<PollHistogram, ClassroomError> {
        let histogram = self
            .polls
            .get_mut(poll_id)
            .ok_or_else(|| ClassroomError::Poll(format!("unknown poll {poll_id}")))?
            .close();
        self.transport.send(&ClassroomPacket::PollClosed {
            poll_id: poll_id.to_owned(),
            histogram: histogram.clone(),
        })?;
        Ok(histogram)
    }

    /// Reads current live histogram for `poll_id`.
    #[must_use]
    pub fn poll_histogram(&self, poll_id: &str) -> Option<PollHistogram> {
        self.polls.get(poll_id).map(PollTally::histogram)
    }

    /// Processes incoming student join requests and poll votes from the transport.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError`] if reading or sending fails.
    pub fn process_incoming(&mut self) -> Result<usize, ClassroomError> {
        let packets = self.transport.drain()?;
        let count = packets.len();
        for pkt in packets {
            match pkt {
                ClassroomPacket::JoinRequest {
                    code,
                    student_id,
                    display_name,
                } => {
                    if code == self.join_code {
                        self.students.insert(
                            student_id.clone(),
                            ConnectedStudent {
                                student_id: student_id.clone(),
                                display_name,
                            },
                        );
                        self.transport.send(&ClassroomPacket::JoinAccepted {
                            student_id,
                            session_title: self.session_title.clone(),
                            current_focus: self.current_focus.clone(),
                        })?;
                    }
                }
                ClassroomPacket::PollVoteSubmitted(vote) => {
                    if let Some(tally) = self.polls.get_mut(&vote.poll_id) {
                        let _ = tally.record_vote(&vote);
                    }
                }
                _ => {}
            }
        }
        Ok(count)
    }
}

/// Student classroom client receiving live teacher focus and answering quick polls (plan G.14, G.15).
pub struct ClassroomStudent<T: ClassroomTransport> {
    student_id: String,
    display_name: Option<String>,
    transport: T,
    joined_session: Option<String>,
    follow_mode: FollowMode,
    followed_focus: Option<ReadingFocus>,
    active_polls: BTreeMap<String, QuickPoll>,
    closed_histograms: BTreeMap<String, PollHistogram>,
}

impl<T: ClassroomTransport> ClassroomStudent<T> {
    /// Creates a student client with an ephemeral `student_id` (no account needed).
    #[must_use]
    pub fn new(student_id: &str, display_name: Option<&str>, transport: T) -> Self {
        Self {
            student_id: student_id.to_owned(),
            display_name: display_name.map(str::to_owned),
            transport,
            joined_session: None,
            follow_mode: FollowMode::LockedToTeacher,
            followed_focus: None,
            active_polls: BTreeMap::new(),
            closed_histograms: BTreeMap::new(),
        }
    }

    /// Sets whether this student's viewport automatically tracks teacher focus updates.
    pub fn set_follow_mode(&mut self, mode: FollowMode) {
        self.follow_mode = mode;
    }

    /// Sends a join request for `code`.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError`] if sending fails.
    pub fn join(&mut self, code: &JoinCode) -> Result<(), ClassroomError> {
        self.transport.send(&ClassroomPacket::JoinRequest {
            code: code.clone(),
            student_id: self.student_id.clone(),
            display_name: self.display_name.clone(),
        })
    }

    /// Submits a vote for `poll_id` and `option_index`.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError`] if sending fails.
    pub fn submit_vote(
        &mut self,
        poll_id: &str,
        option_index: usize,
    ) -> Result<(), ClassroomError> {
        let vote = self::poll::PollVote {
            poll_id: poll_id.to_owned(),
            option_index,
            voter_token: self.student_id.clone(),
            voter_name: self.display_name.clone(),
        };
        self.transport
            .send(&ClassroomPacket::PollVoteSubmitted(vote))
    }

    /// Polls incoming classroom packets and updates followed focus / polls.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError`] if draining fails.
    pub fn sync_updates(&mut self) -> Result<usize, ClassroomError> {
        let packets = self.transport.drain()?;
        let count = packets.len();
        for pkt in packets {
            match pkt {
                ClassroomPacket::JoinAccepted {
                    student_id,
                    session_title,
                    current_focus,
                } if student_id == self.student_id => {
                    self.joined_session = Some(session_title);
                    if matches!(self.follow_mode, FollowMode::LockedToTeacher)
                        && let Some(focus) = current_focus
                    {
                        self.followed_focus = Some(focus);
                    }
                }
                ClassroomPacket::FocusUpdate(focus) => {
                    if matches!(self.follow_mode, FollowMode::LockedToTeacher) {
                        self.followed_focus = Some(focus);
                    }
                }
                ClassroomPacket::PollPublished(poll) => {
                    self.active_polls.insert(poll.id.clone(), poll);
                }
                ClassroomPacket::PollClosed { poll_id, histogram } => {
                    self.active_polls.remove(&poll_id);
                    self.closed_histograms.insert(poll_id, histogram);
                }
                _ => {}
            }
        }
        Ok(count)
    }

    /// Latest teacher [`ReadingFocus`] received by this student.
    #[must_use]
    pub const fn current_focus(&self) -> Option<&ReadingFocus> {
        self.followed_focus.as_ref()
    }

    /// Joined classroom session title, if accepted.
    #[must_use]
    pub fn session_title(&self) -> Option<&str> {
        self.joined_session.as_deref()
    }

    /// Active open poll by id.
    #[must_use]
    pub fn active_poll(&self, poll_id: &str) -> Option<&QuickPoll> {
        self.active_polls.get(poll_id)
    }

    /// Closed poll histogram by id.
    #[must_use]
    pub fn closed_histogram(&self, poll_id: &str) -> Option<&PollHistogram> {
        self.closed_histograms.get(poll_id)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::poll::VotePrivacy;
    use super::transport::{InMemoryClassroomBus, TcpClassroomClient, TcpClassroomServer};
    use super::*;

    #[test]
    fn thirty_students_follow_teacher_page_changes_well_under_300ms() {
        let bus = InMemoryClassroomBus::new();
        let code = JoinCode::from_seed(2026);
        let mut host = ClassroomHost::new("Algorithms 101", code.clone(), bus.host_transport());

        let beacon = host.discovery_beacon("192.168.1.10:9420", None);
        let txt = beacon.to_txt_records();
        let parsed_beacon = DiscoveryBeacon::from_txt_records(&txt).expect("parse beacon");
        assert_eq!(parsed_beacon.join_code, code);

        let mut students: Vec<_> = (0..30)
            .map(|i| {
                let mut s = ClassroomStudent::new(
                    &format!("stu-{i:02}"),
                    Some(&format!("Student {i}")),
                    bus.connect_student(),
                );
                s.join(&code).expect("join");
                s
            })
            .collect();

        host.process_incoming().expect("accept joins");
        assert_eq!(host.student_count(), 30);

        let start = Instant::now();
        host.broadcast_focus("lecture.pdf", 14, 120.0, 1.25, Some((0.5, 0.3)))
            .expect("broadcast");
        for student in &mut students {
            student.sync_updates().expect("sync");
            let focus = student.current_focus().expect("student received focus");
            assert_eq!(focus.page, 14);
            assert_eq!(focus.doc_id, "lecture.pdf");
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 300,
            "30 students synced in {elapsed:?}"
        );
    }

    #[test]
    fn quick_polls_are_anonymous_by_default_and_aggregate_histograms() {
        let bus = InMemoryClassroomBus::new();
        let code = JoinCode::try_new("CS202").expect("code");
        let mut host = ClassroomHost::new("Systems Seminar", code.clone(), bus.host_transport());

        let mut s1 = ClassroomStudent::new("tok-1", Some("Alice"), bus.connect_student());
        let mut s2 = ClassroomStudent::new("tok-2", Some("Bob"), bus.connect_student());
        let mut s3 = ClassroomStudent::new("tok-3", Some("Cara"), bus.connect_student());
        s1.join(&code).expect("j1");
        s2.join(&code).expect("j2");
        s3.join(&code).expect("j3");
        host.process_incoming().expect("joins");

        let poll = QuickPoll::new(
            "p1",
            "Which CRDT handles concurrent text edits?",
            &["LWW Register", "RGA Sequence", "G-Counter"],
        );
        assert_eq!(poll.privacy, VotePrivacy::Anonymous);
        host.publish_poll(poll).expect("publish");

        s1.sync_updates().expect("s1");
        s2.sync_updates().expect("s2");
        s3.sync_updates().expect("s3");

        s1.submit_vote("p1", 1).expect("v1");
        s2.submit_vote("p1", 1).expect("v2");
        s3.submit_vote("p1", 0).expect("v3");
        host.process_incoming().expect("tally");

        let hist = host.close_poll("p1").expect("close poll");
        assert_eq!(hist.total_votes, 3);
        assert_eq!(hist.bars[0].votes, 1);
        assert_eq!(hist.bars[1].votes, 2);
        assert_eq!(
            hist.named_voters, None,
            "anonymous poll must never expose voter names"
        );

        s1.sync_updates().expect("s1 closed");
        assert_eq!(s1.closed_histogram("p1").expect("hist").total_votes, 3);
    }

    #[test]
    fn local_tcp_classroom_transport_delivers_focus_updates() {
        let server = TcpClassroomServer::bind("127.0.0.1:0").expect("bind tcp");
        let addr = server.local_addr();
        let code = JoinCode::try_new("LAN99").expect("code");
        let mut host = ClassroomHost::new("LAN Class", code.clone(), server);

        let client = TcpClassroomClient::connect(addr).expect("connect tcp");
        let mut student = ClassroomStudent::new("lan-1", None, client);
        student.join(&code).expect("join over tcp");

        // Allow loopback socket accept + flush.
        std::thread::sleep(std::time::Duration::from_millis(20));
        host.process_incoming().expect("host process");
        host.broadcast_focus("slides.pdf", 5, 0.0, 1.0, None)
            .expect("broadcast");
        std::thread::sleep(std::time::Duration::from_millis(20));
        student.sync_updates().expect("student sync");

        assert_eq!(student.current_focus().expect("focus").page, 5);
    }
}

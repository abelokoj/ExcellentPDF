//! Wire protocol messages, join codes, mDNS discovery beacons, and focus state (plan G.14).

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Value, json};

use super::ClassroomError;
use super::poll::{PollHistogram, PollVote, QuickPoll};

/// DNS-SD / mDNS service type advertised by LAN classroom hosts (plan G.14).
pub const MDNS_SERVICE_TYPE: &str = "_ecclesiapdf-class._tcp.local";

/// Six-character uppercase alphanumeric classroom join code (no account required).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JoinCode(String);

impl JoinCode {
    /// Validates and constructs a [`JoinCode`] (4..=12 ASCII alphanumeric chars, uppercased).
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError::InvalidCode`] if the code is empty, too short/long, or contains non-alphanumeric chars.
    pub fn try_new(raw: &str) -> Result<Self, ClassroomError> {
        let cleaned: String = raw.trim().to_ascii_uppercase();
        if !(4..=12).contains(&cleaned.len()) || !cleaned.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return Err(ClassroomError::InvalidCode(raw.to_owned()));
        }
        Ok(Self(cleaned))
    }

    /// Deterministically derives a readable 6-character join code from `seed`.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
        let mut s = seed ^ 0x517C_C1B7_2722_0A95;
        let mut out = String::with_capacity(6);
        for _ in 0..6 {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let idx = ((s >> 32) as usize) % ALPHABET.len();
            out.push(char::from(ALPHABET[idx]));
        }
        Self(out)
    }

    /// Borrowed code slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for JoinCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Teacher's live reading position and optional laser pointer broadcast to students (plan G.14).
#[derive(Debug, Clone, PartialEq)]
pub struct ReadingFocus {
    /// Active document identifier or filename.
    pub doc_id: String,
    /// Zero-based page index.
    pub page: usize,
    /// Vertical scroll offset in page points.
    pub scroll_y: f32,
    /// Zoom factor.
    pub zoom: f32,
    /// Optional normalized laser pointer `(x, y)` in `0.0..=1.0` page coordinates.
    pub laser_point: Option<(f32, f32)>,
    /// Monotonic sequence number assigned by the teacher host.
    pub seq: u64,
}

/// Student viewport synchronization preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum FollowMode {
    /// Automatically follow every teacher page/scroll update.
    #[default]
    LockedToTeacher,
    /// Browse independently while remaining connected to polls and updates.
    FreeBrowse,
}

/// mDNS/DNS-SD discovery advertisement emitted by a LAN classroom host (plan G.14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryBeacon {
    /// Human-readable session title.
    pub session_title: String,
    /// Join code required to enter.
    pub join_code: JoinCode,
    /// LAN TCP socket address (`ip:port`).
    pub host_addr: String,
    /// Optional self-hosted HTTP relay base URL.
    pub relay_url: Option<String>,
}

impl DiscoveryBeacon {
    /// Encodes TXT key/value pairs for mDNS DNS-SD announcement.
    #[must_use]
    pub fn to_txt_records(&self) -> BTreeMap<String, String> {
        let mut txt = BTreeMap::new();
        txt.insert("svc".to_owned(), MDNS_SERVICE_TYPE.to_owned());
        txt.insert("title".to_owned(), self.session_title.clone());
        txt.insert("code".to_owned(), self.join_code.as_str().to_owned());
        txt.insert("addr".to_owned(), self.host_addr.clone());
        if let Some(relay) = &self.relay_url {
            txt.insert("relay".to_owned(), relay.clone());
        }
        txt
    }

    /// Parses a [`DiscoveryBeacon`] from mDNS TXT records.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError::Protocol`] if required TXT keys are missing or invalid.
    pub fn from_txt_records(txt: &BTreeMap<String, String>) -> Result<Self, ClassroomError> {
        let session_title = txt
            .get("title")
            .cloned()
            .ok_or_else(|| ClassroomError::Protocol("beacon missing title".to_owned()))?;
        let code_raw = txt
            .get("code")
            .ok_or_else(|| ClassroomError::Protocol("beacon missing code".to_owned()))?;
        let host_addr = txt
            .get("addr")
            .cloned()
            .ok_or_else(|| ClassroomError::Protocol("beacon missing addr".to_owned()))?;
        Ok(Self {
            session_title,
            join_code: JoinCode::try_new(code_raw)?,
            host_addr,
            relay_url: txt.get("relay").cloned(),
        })
    }
}

/// Wire packet exchanged between [`super::ClassroomHost`] and [`super::ClassroomStudent`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ClassroomPacket {
    /// Student request to join by code (no account).
    JoinRequest {
        /// Join code entered by student.
        code: JoinCode,
        /// Ephemeral student session token.
        student_id: String,
        /// Optional display name.
        display_name: Option<String>,
    },
    /// Host confirmation with current focus.
    JoinAccepted {
        /// Student id acknowledged.
        student_id: String,
        /// Session title.
        session_title: String,
        /// Current teacher focus, if a document is shared.
        current_focus: Option<ReadingFocus>,
    },
    /// Teacher pushed a slide/page/laser focus update.
    FocusUpdate(ReadingFocus),
    /// Teacher published a quick poll (plan G.15).
    PollPublished(QuickPoll),
    /// Student submitted a quick poll vote (plan G.15).
    PollVoteSubmitted(PollVote),
    /// Teacher closed a quick poll with final histogram.
    PollClosed {
        /// Closed poll id.
        poll_id: String,
        /// Aggregated results.
        histogram: PollHistogram,
    },
    /// Host ended the live classroom session.
    SessionEnded,
}

impl ClassroomPacket {
    /// Serializes packet to a single-line JSON string.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError::Protocol`] if serialization fails.
    pub fn to_json_line(&self) -> Result<String, ClassroomError> {
        let val = match self {
            Self::JoinRequest {
                code,
                student_id,
                display_name,
            } => json!({
                "type": "join_request",
                "code": code.as_str(),
                "student_id": student_id,
                "display_name": display_name,
            }),
            Self::JoinAccepted {
                student_id,
                session_title,
                current_focus,
            } => json!({
                "type": "join_accepted",
                "student_id": student_id,
                "session_title": session_title,
                "current_focus": current_focus.as_ref().map(focus_to_json),
            }),
            Self::FocusUpdate(focus) => json!({
                "type": "focus_update",
                "focus": focus_to_json(focus),
            }),
            Self::PollPublished(poll) => json!({
                "type": "poll_published",
                "poll": poll.to_json(),
            }),
            Self::PollVoteSubmitted(vote) => json!({
                "type": "poll_vote",
                "vote": vote.to_json(),
            }),
            Self::PollClosed { poll_id, histogram } => json!({
                "type": "poll_closed",
                "poll_id": poll_id,
                "histogram": histogram.to_json(),
            }),
            Self::SessionEnded => json!({ "type": "session_ended" }),
        };
        serde_json::to_string(&val).map_err(|e| ClassroomError::Protocol(e.to_string()))
    }

    /// Parses packet from a JSON line.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError::Protocol`] if the line is invalid JSON or has an unknown type.
    pub fn from_json_line(line: &str) -> Result<Self, ClassroomError> {
        let v: Value = serde_json::from_str(line)
            .map_err(|e| ClassroomError::Protocol(format!("invalid packet json: {e}")))?;
        let ty = v
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| ClassroomError::Protocol("packet missing type".to_owned()))?;
        match ty {
            "join_request" => Ok(Self::JoinRequest {
                code: JoinCode::try_new(
                    v.get("code")
                        .and_then(Value::as_str)
                        .ok_or_else(|| ClassroomError::Protocol("missing code".to_owned()))?,
                )?,
                student_id: v
                    .get("student_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ClassroomError::Protocol("missing student_id".to_owned()))?
                    .to_owned(),
                display_name: v
                    .get("display_name")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }),
            "join_accepted" => Ok(Self::JoinAccepted {
                student_id: v
                    .get("student_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                session_title: v
                    .get("session_title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                current_focus: v
                    .get("current_focus")
                    .filter(|f| !f.is_null())
                    .map(focus_from_json)
                    .transpose()?,
            }),
            "focus_update" => Ok(Self::FocusUpdate(focus_from_json(
                v.get("focus")
                    .ok_or_else(|| ClassroomError::Protocol("missing focus".to_owned()))?,
            )?)),
            "poll_published" => Ok(Self::PollPublished(QuickPoll::from_json(
                v.get("poll")
                    .ok_or_else(|| ClassroomError::Protocol("missing poll".to_owned()))?,
            )?)),
            "poll_vote" => Ok(Self::PollVoteSubmitted(PollVote::from_json(
                v.get("vote")
                    .ok_or_else(|| ClassroomError::Protocol("missing vote".to_owned()))?,
            )?)),
            "poll_closed" => Ok(Self::PollClosed {
                poll_id: v
                    .get("poll_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ClassroomError::Protocol("missing poll_id".to_owned()))?
                    .to_owned(),
                histogram: PollHistogram::from_json(
                    v.get("histogram")
                        .ok_or_else(|| ClassroomError::Protocol("missing histogram".to_owned()))?,
                )?,
            }),
            "session_ended" => Ok(Self::SessionEnded),
            other => Err(ClassroomError::Protocol(format!(
                "unknown packet type {other}"
            ))),
        }
    }
}

fn focus_to_json(f: &ReadingFocus) -> Value {
    json!({
        "doc_id": f.doc_id,
        "page": f.page,
        "scroll_y": f.scroll_y,
        "zoom": f.zoom,
        "laser_point": f.laser_point.map(|(x, y)| json!([x, y])),
        "seq": f.seq,
    })
}

fn focus_from_json(v: &Value) -> Result<ReadingFocus, ClassroomError> {
    let laser_point = v.get("laser_point").and_then(|lp| {
        let arr = lp.as_array()?;
        Some((arr.first()?.as_f64()? as f32, arr.get(1)?.as_f64()? as f32))
    });
    Ok(ReadingFocus {
        doc_id: v
            .get("doc_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ClassroomError::Protocol("focus missing doc_id".to_owned()))?
            .to_owned(),
        page: v
            .get("page")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0),
        scroll_y: v.get("scroll_y").and_then(Value::as_f64).unwrap_or(0.0) as f32,
        zoom: v.get("zoom").and_then(Value::as_f64).unwrap_or(1.0) as f32,
        laser_point,
        seq: v.get("seq").and_then(Value::as_u64).unwrap_or(0),
    })
}

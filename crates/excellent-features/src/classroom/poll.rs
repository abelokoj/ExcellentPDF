//! Live classroom quick polls: anonymous-by-default voting and instant histogram aggregation (plan G.15).

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::ClassroomError;

/// Controls whether voter display names are recorded in poll results (plan G.15: anonymous by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum VotePrivacy {
    /// Voter names are stripped and omitted from [`PollHistogram`].
    #[default]
    Anonymous,
    /// Voter names are retained alongside option tallies.
    Named,
}

impl VotePrivacy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::Named => "named",
        }
    }

    fn from_str(raw: &str) -> Self {
        match raw {
            "named" => Self::Named,
            _ => Self::Anonymous,
        }
    }
}

/// Lifecycle state of a [`QuickPoll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PollStatus {
    /// Accepting student votes.
    #[default]
    Open,
    /// Closed; votes are frozen.
    Closed,
}

/// One answer choice in a [`QuickPoll`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollOption {
    /// Zero-based option index.
    pub index: usize,
    /// Option label text.
    pub label: String,
}

/// Multiple-choice question broadcast by a teacher host (plan G.15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickPoll {
    /// Unique poll identifier.
    pub id: String,
    /// Prompt question text.
    pub question: String,
    /// Available answer options.
    pub options: Vec<PollOption>,
    /// Privacy mode (defaults to [`VotePrivacy::Anonymous`]).
    pub privacy: VotePrivacy,
    /// Current status.
    pub status: PollStatus,
}

impl QuickPoll {
    /// Creates an open anonymous quick poll from `id`, `question`, and `option_labels`.
    #[must_use]
    pub fn new(id: &str, question: &str, option_labels: &[&str]) -> Self {
        Self::with_privacy(id, question, option_labels, VotePrivacy::Anonymous)
    }

    /// Creates an open quick poll with explicit [`VotePrivacy`].
    #[must_use]
    pub fn with_privacy(
        id: &str,
        question: &str,
        option_labels: &[&str],
        privacy: VotePrivacy,
    ) -> Self {
        Self {
            id: id.to_owned(),
            question: question.to_owned(),
            options: option_labels
                .iter()
                .enumerate()
                .map(|(index, label)| PollOption {
                    index,
                    label: (*label).to_owned(),
                })
                .collect(),
            privacy,
            status: PollStatus::Open,
        }
    }

    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "question": self.question,
            "options": self.options.iter().map(|o| json!({ "index": o.index, "label": o.label })).collect::<Vec<_>>(),
            "privacy": self.privacy.as_str(),
            "status": match self.status { PollStatus::Open => "open", PollStatus::Closed => "closed" },
        })
    }

    /// Deserializes from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError::Protocol`] if fields are malformed.
    pub fn from_json(v: &Value) -> Result<Self, ClassroomError> {
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ClassroomError::Protocol("poll missing id".to_owned()))?
            .to_owned();
        let question = v
            .get("question")
            .and_then(Value::as_str)
            .ok_or_else(|| ClassroomError::Protocol("poll missing question".to_owned()))?
            .to_owned();
        let arr = v
            .get("options")
            .and_then(Value::as_array)
            .ok_or_else(|| ClassroomError::Protocol("poll missing options".to_owned()))?;
        let mut options = Vec::with_capacity(arr.len());
        for (fallback_idx, item) in arr.iter().enumerate() {
            let index = item
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(fallback_idx);
            let label = item
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            options.push(PollOption { index, label });
        }
        let privacy = VotePrivacy::from_str(
            v.get("privacy")
                .and_then(Value::as_str)
                .unwrap_or("anonymous"),
        );
        let status = match v.get("status").and_then(Value::as_str) {
            Some("closed") => PollStatus::Closed,
            _ => PollStatus::Open,
        };
        Ok(Self {
            id,
            question,
            options,
            privacy,
            status,
        })
    }
}

/// Student's vote submission for a [`QuickPoll`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollVote {
    /// Target poll id.
    pub poll_id: String,
    /// Chosen option index.
    pub option_index: usize,
    /// Opaque per-student token used only to prevent duplicate counting.
    pub voter_token: String,
    /// Optional voter display name (ignored unless poll is [`VotePrivacy::Named`]).
    pub voter_name: Option<String>,
}

impl PollVote {
    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "poll_id": self.poll_id,
            "option_index": self.option_index,
            "voter_token": self.voter_token,
            "voter_name": self.voter_name,
        })
    }

    /// Deserializes from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError::Protocol`] if required fields are missing.
    pub fn from_json(v: &Value) -> Result<Self, ClassroomError> {
        Ok(Self {
            poll_id: v
                .get("poll_id")
                .and_then(Value::as_str)
                .ok_or_else(|| ClassroomError::Protocol("vote missing poll_id".to_owned()))?
                .to_owned(),
            option_index: v
                .get("option_index")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| ClassroomError::Protocol("vote missing option_index".to_owned()))?,
            voter_token: v
                .get("voter_token")
                .and_then(Value::as_str)
                .ok_or_else(|| ClassroomError::Protocol("vote missing voter_token".to_owned()))?
                .to_owned(),
            voter_name: v
                .get("voter_name")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
}

/// One bar in a [`PollHistogram`] suitable for plotting with NSH `Plot`.
#[derive(Debug, Clone, PartialEq)]
pub struct PollHistogramBar {
    /// Option index.
    pub option_index: usize,
    /// Option label.
    pub label: String,
    /// Vote count for this option.
    pub votes: usize,
    /// Percentage in `0.0..=100.0`.
    pub percentage: f64,
}

/// Aggregated poll result distribution (plan G.15).
#[derive(Debug, Clone, PartialEq)]
pub struct PollHistogram {
    /// Poll id.
    pub poll_id: String,
    /// Question prompt.
    pub question: String,
    /// Total distinct votes cast.
    pub total_votes: usize,
    /// Effective privacy policy (`named_voters` is `None` when [`VotePrivacy::Anonymous`]).
    pub privacy: VotePrivacy,
    /// Ordered bars matching the poll's options.
    pub bars: Vec<PollHistogramBar>,
    /// Voter display names per option index when [`VotePrivacy::Named`]; always `None` when anonymous.
    pub named_voters: Option<BTreeMap<usize, Vec<String>>>,
}

impl PollHistogram {
    /// Serializes to JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "poll_id": self.poll_id,
            "question": self.question,
            "total_votes": self.total_votes,
            "privacy": self.privacy.as_str(),
            "bars": self.bars.iter().map(|b| json!({
                "option_index": b.option_index,
                "label": b.label,
                "votes": b.votes,
                "percentage": b.percentage,
            })).collect::<Vec<_>>(),
            "named_voters": self.named_voters,
        })
    }

    /// Deserializes from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError::Protocol`] if fields are malformed.
    pub fn from_json(v: &Value) -> Result<Self, ClassroomError> {
        let poll_id = v
            .get("poll_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ClassroomError::Protocol("histogram missing poll_id".to_owned()))?
            .to_owned();
        let question = v
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let total_votes = v
            .get("total_votes")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0);
        let privacy = VotePrivacy::from_str(
            v.get("privacy")
                .and_then(Value::as_str)
                .unwrap_or("anonymous"),
        );
        let mut bars = Vec::new();
        if let Some(arr) = v.get("bars").and_then(Value::as_array) {
            for b in arr {
                bars.push(PollHistogramBar {
                    option_index: b
                        .get("option_index")
                        .and_then(Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok())
                        .unwrap_or(0),
                    label: b
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    votes: b
                        .get("votes")
                        .and_then(Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok())
                        .unwrap_or(0),
                    percentage: b.get("percentage").and_then(Value::as_f64).unwrap_or(0.0),
                });
            }
        }
        let named_voters = if matches!(privacy, VotePrivacy::Named) {
            v.get("named_voters").and_then(Value::as_object).map(|obj| {
                obj.iter()
                    .filter_map(|(k, list)| {
                        let idx = k.parse::<usize>().ok()?;
                        let names = list
                            .as_array()?
                            .iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect();
                        Some((idx, names))
                    })
                    .collect()
            })
        } else {
            None
        };
        Ok(Self {
            poll_id,
            question,
            total_votes,
            privacy,
            bars,
            named_voters,
        })
    }
}

/// Server-side poll state collecting student votes and producing [`PollHistogram`]s (plan G.15).
#[derive(Debug, Clone)]
pub struct PollTally {
    poll: QuickPoll,
    votes_by_token: BTreeMap<String, (usize, Option<String>)>,
}

impl PollTally {
    /// Starts tallying votes for `poll`.
    #[must_use]
    pub fn new(poll: QuickPoll) -> Self {
        Self {
            poll,
            votes_by_token: BTreeMap::new(),
        }
    }

    /// Underlying [`QuickPoll`] definition.
    #[must_use]
    pub const fn poll(&self) -> &QuickPoll {
        &self.poll
    }

    /// Records or updates a student's vote, enforcing anonymous privacy by stripping voter names
    /// whenever [`VotePrivacy::Anonymous`] is active.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError::Poll`] if the poll is closed or `option_index` is out of bounds.
    pub fn record_vote(&mut self, vote: &PollVote) -> Result<(), ClassroomError> {
        if vote.poll_id != self.poll.id {
            return Err(ClassroomError::Poll(format!(
                "vote poll_id {} does not match {}",
                vote.poll_id, self.poll.id
            )));
        }
        if matches!(self.poll.status, PollStatus::Closed) {
            return Err(ClassroomError::Poll("poll is closed".to_owned()));
        }
        if vote.option_index >= self.poll.options.len() {
            return Err(ClassroomError::Poll(format!(
                "option index {} out of range",
                vote.option_index
            )));
        }
        let stored_name = match self.poll.privacy {
            VotePrivacy::Anonymous => None,
            VotePrivacy::Named => vote.voter_name.clone(),
        };
        self.votes_by_token
            .insert(vote.voter_token.clone(), (vote.option_index, stored_name));
        Ok(())
    }

    /// Closes the poll and returns its final [`PollHistogram`].
    #[must_use]
    pub fn close(&mut self) -> PollHistogram {
        self.poll.status = PollStatus::Closed;
        self.histogram()
    }

    /// Computes the live [`PollHistogram`] bar chart data.
    #[must_use]
    pub fn histogram(&self) -> PollHistogram {
        let mut counts = vec![0_usize; self.poll.options.len()];
        let mut named = BTreeMap::<usize, Vec<String>>::new();
        for (idx, name) in self.votes_by_token.values() {
            if let Some(slot) = counts.get_mut(*idx) {
                *slot += 1;
            }
            if matches!(self.poll.privacy, VotePrivacy::Named)
                && let Some(voter) = name
            {
                named.entry(*idx).or_default().push(voter.clone());
            }
        }
        let total = self.votes_by_token.len();
        let bars = self
            .poll
            .options
            .iter()
            .enumerate()
            .map(|(i, opt)| {
                let c = counts.get(i).copied().unwrap_or(0);
                let pct = if total == 0 {
                    0.0
                } else {
                    (c as f64 / total as f64) * 100.0
                };
                PollHistogramBar {
                    option_index: opt.index,
                    label: opt.label.clone(),
                    votes: c,
                    percentage: pct,
                }
            })
            .collect();

        PollHistogram {
            poll_id: self.poll.id.clone(),
            question: self.poll.question.clone(),
            total_votes: total,
            privacy: self.poll.privacy,
            bars,
            named_voters: matches!(self.poll.privacy, VotePrivacy::Named).then_some(named),
        }
    }
}

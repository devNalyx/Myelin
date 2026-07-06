use chrono::Utc;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

/// Reps a candidate needs (with no high-stakes signal) before it auto-promotes.
/// A guess, not a measured value — see README's open risks. Easy to find here
/// and retune once there's real usage data.
pub const PROMOTION_REPS: i64 = 3;

/// Token-overlap (Jaccard) threshold for "this observation is the same
/// procedure as that candidate". Crude on purpose for MVP — no embeddings
/// dependency required. Swap for real semantic similarity once this proves
/// too blunt in practice.
pub const SIMILARITY_THRESHOLD: f64 = 0.4;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS skill_candidates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL,
    title TEXT NOT NULL,
    rep_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'warming',
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS observations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    project TEXT,
    context_signal TEXT,
    high_stakes INTEGER NOT NULL DEFAULT 0,
    candidate_id INTEGER NOT NULL REFERENCES skill_candidates(id)
);

CREATE TABLE IF NOT EXISTS skills (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    candidate_id INTEGER NOT NULL REFERENCES skill_candidates(id),
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    path TEXT NOT NULL,
    promoted_reason TEXT NOT NULL,
    observation_count INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS corrections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    skill_id INTEGER NOT NULL REFERENCES skills(id),
    created_at TEXT NOT NULL,
    kind TEXT NOT NULL,
    note TEXT NOT NULL
);
";

pub struct Store {
    conn: Connection,
}

pub struct NewObservation {
    pub title: String,
    pub summary: String,
    pub project: Option<String>,
    pub context_signal: Option<String>,
    pub high_stakes: bool,
}

#[derive(Debug, Serialize)]
pub struct RecordResult {
    pub candidate_id: i64,
    pub rep_count: i64,
    pub promoted: bool,
    pub skill_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CandidateView {
    pub id: i64,
    pub title: String,
    pub rep_count: i64,
    pub status: String,
    pub first_seen: String,
    pub last_seen: String,
}

#[derive(Debug, Serialize)]
pub struct SkillView {
    pub id: i64,
    pub candidate_id: i64,
    pub slug: String,
    pub name: String,
    pub path: String,
    pub promoted_reason: String,
    pub observation_count: i64,
    pub created_at: String,
    pub correction_count: i64,
    pub confirmation_count: i64,
}

#[derive(Debug, Serialize)]
pub struct FeedbackResult {
    pub skill_id: i64,
    pub correction_count: i64,
    pub confirmation_count: i64,
}

fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| s.len() > 2)
        .map(str::to_string)
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    intersection / union
}

impl Store {
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Records one observation, matches/creates its candidate, and promotes
    /// the candidate to a real skill if this observation crosses a
    /// promotion trigger (reps threshold, or an explicit high-stakes flag).
    pub fn record_observation(
        &self,
        input: NewObservation,
        skills_dir: &Path,
    ) -> anyhow::Result<RecordResult> {
        let tokens = tokenize(&format!("{} {}", input.title, input.summary));
        let now = Utc::now().to_rfc3339();

        let mut best: Option<(i64, f64, i64, String)> = None;
        {
            let mut stmt = self
                .conn
                .prepare("SELECT id, key, rep_count, status FROM skill_candidates")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            for row in rows {
                let (id, key, rep_count, status) = row?;
                let cand_tokens: HashSet<String> =
                    key.split(' ').filter(|s| !s.is_empty()).map(String::from).collect();
                let score = jaccard(&tokens, &cand_tokens);
                if score >= SIMILARITY_THRESHOLD
                    && best.as_ref().map(|b| score > b.1).unwrap_or(true)
                {
                    best = Some((id, score, rep_count, status));
                }
            }
        }

        let (candidate_id, rep_count, status) = if let Some((id, _, rep_count, status)) = best {
            self.conn.execute(
                "UPDATE skill_candidates SET rep_count = rep_count + 1, last_seen = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            (id, rep_count + 1, status)
        } else {
            let mut sorted_tokens: Vec<_> = tokens.iter().cloned().collect();
            sorted_tokens.sort();
            let key = sorted_tokens.join(" ");
            self.conn.execute(
                "INSERT INTO skill_candidates (key, title, rep_count, status, first_seen, last_seen)
                 VALUES (?1, ?2, 1, 'warming', ?3, ?3)",
                params![key, input.title, now],
            )?;
            (self.conn.last_insert_rowid(), 1, "warming".to_string())
        };

        self.conn.execute(
            "INSERT INTO observations
             (created_at, title, summary, project, context_signal, high_stakes, candidate_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                now,
                input.title,
                input.summary,
                input.project,
                input.context_signal,
                input.high_stakes,
                candidate_id
            ],
        )?;

        let mut promoted = false;
        let mut skill_path = None;

        if status == "warming" && (input.high_stakes || rep_count >= PROMOTION_REPS) {
            let reason = if input.high_stakes { "context_signal" } else { "reps" };
            skill_path = Some(self.promote_internal(candidate_id, reason, skills_dir)?);
            promoted = true;
        }

        Ok(RecordResult {
            candidate_id,
            rep_count,
            promoted,
            skill_path,
        })
    }

    /// Force-promotes a candidate regardless of reps/high-stakes state.
    /// Errors if it's already promoted.
    pub fn promote_candidate(&self, candidate_id: i64, skills_dir: &Path) -> anyhow::Result<String> {
        let status: String = self
            .conn
            .query_row(
                "SELECT status FROM skill_candidates WHERE id = ?1",
                params![candidate_id],
                |r| r.get(0),
            )
            .map_err(|_| anyhow::anyhow!("no such candidate: {candidate_id}"))?;
        if status == "promoted" {
            anyhow::bail!("candidate {candidate_id} is already promoted");
        }
        self.promote_internal(candidate_id, "manual", skills_dir)
    }

    fn promote_internal(
        &self,
        candidate_id: i64,
        reason: &str,
        skills_dir: &Path,
    ) -> anyhow::Result<String> {
        let title: String = self.conn.query_row(
            "SELECT title FROM skill_candidates WHERE id = ?1",
            params![candidate_id],
            |r| r.get(0),
        )?;

        let mut stmt = self
            .conn
            .prepare("SELECT summary, project FROM observations WHERE candidate_id = ?1 ORDER BY id")?;
        let summaries: Vec<(String, Option<String>)> = stmt
            .query_map(params![candidate_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<_, _>>()?;

        let (slug, path) = crate::skillfile::draft_and_write(&title, &summaries, reason, skills_dir)?;
        let now = Utc::now().to_rfc3339();
        let path_str = path.to_string_lossy().to_string();

        self.conn.execute(
            "INSERT INTO skills
             (candidate_id, slug, name, path, promoted_reason, observation_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                candidate_id,
                slug,
                title,
                path_str,
                reason,
                summaries.len() as i64,
                now
            ],
        )?;
        self.conn.execute(
            "UPDATE skill_candidates SET status = 'promoted' WHERE id = ?1",
            params![candidate_id],
        )?;

        Ok(path_str)
    }

    pub fn list_candidates(&self) -> anyhow::Result<Vec<CandidateView>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, rep_count, status, first_seen, last_seen
             FROM skill_candidates ORDER BY last_seen DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(CandidateView {
                id: row.get(0)?,
                title: row.get(1)?,
                rep_count: row.get(2)?,
                status: row.get(3)?,
                first_seen: row.get(4)?,
                last_seen: row.get(5)?,
            })
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    pub fn list_skills(&self) -> anyhow::Result<Vec<SkillView>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.candidate_id, s.slug, s.name, s.path, s.promoted_reason,
                    s.observation_count, s.created_at,
                    (SELECT COUNT(*) FROM corrections c WHERE c.skill_id = s.id AND c.kind = 'correction'),
                    (SELECT COUNT(*) FROM corrections c WHERE c.skill_id = s.id AND c.kind = 'confirmation')
             FROM skills s ORDER BY s.created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SkillView {
                id: row.get(0)?,
                candidate_id: row.get(1)?,
                slug: row.get(2)?,
                name: row.get(3)?,
                path: row.get(4)?,
                promoted_reason: row.get(5)?,
                observation_count: row.get(6)?,
                created_at: row.get(7)?,
                correction_count: row.get(8)?,
                confirmation_count: row.get(9)?,
            })
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    /// Records feedback on a promoted skill. `kind` is "correction" (the
    /// skill's instructions were wrong/incomplete — appended directly into
    /// the live SKILL.md, so it actually improves) or "confirmation" (it
    /// worked as written — logged for confidence, doesn't touch the file).
    pub fn record_skill_feedback(
        &self,
        skill_id: i64,
        kind: &str,
        note: &str,
    ) -> anyhow::Result<FeedbackResult> {
        if kind != "correction" && kind != "confirmation" {
            anyhow::bail!("kind must be 'correction' or 'confirmation', got '{kind}'");
        }
        let path: String = self
            .conn
            .query_row(
                "SELECT path FROM skills WHERE id = ?1",
                params![skill_id],
                |r| r.get(0),
            )
            .map_err(|_| anyhow::anyhow!("no such skill: {skill_id}"))?;

        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO corrections (skill_id, created_at, kind, note) VALUES (?1, ?2, ?3, ?4)",
            params![skill_id, now, kind, note],
        )?;

        if kind == "correction" {
            crate::skillfile::append_correction(Path::new(&path), note)?;
        }

        let correction_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM corrections WHERE skill_id = ?1 AND kind = 'correction'",
            params![skill_id],
            |r| r.get(0),
        )?;
        let confirmation_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM corrections WHERE skill_id = ?1 AND kind = 'confirmation'",
            params![skill_id],
            |r| r.get(0),
        )?;

        Ok(FeedbackResult {
            skill_id,
            correction_count,
            confirmation_count,
        })
    }
}

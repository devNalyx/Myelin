use chrono::Utc;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

/// Fallback reps threshold when a caller doesn't have a `myelin_core::Config`
/// to load one from (e.g. tests). Kept here rather than depending on
/// myelin-core, to keep this crate's dependency graph shallow. Must match
/// `myelin_core::config::PromotionConfig`'s default.
pub const DEFAULT_PROMOTION_REPS: i64 = 3;

/// Fallback similarity threshold — see `DEFAULT_PROMOTION_REPS`.
pub const DEFAULT_SIMILARITY_THRESHOLD: f64 = 0.4;

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
    promotion_reps: i64,
    similarity_threshold: f64,
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
    /// Opens (creating if needed) the store at `db_path`, with the given
    /// promotion tuning. Use `DEFAULT_PROMOTION_REPS`/`DEFAULT_SIMILARITY_THRESHOLD`
    /// if the caller has no config of its own.
    pub fn open(
        db_path: &Path,
        promotion_reps: i64,
        similarity_threshold: f64,
    ) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn,
            promotion_reps,
            similarity_threshold,
        })
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
                let cand_tokens: HashSet<String> = key
                    .split(' ')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                let score = jaccard(&tokens, &cand_tokens);
                if score >= self.similarity_threshold
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

        if status == "warming" && (input.high_stakes || rep_count >= self.promotion_reps) {
            let reason = if input.high_stakes {
                "context_signal"
            } else {
                "reps"
            };
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
    pub fn promote_candidate(
        &self,
        candidate_id: i64,
        skills_dir: &Path,
    ) -> anyhow::Result<String> {
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

        let mut stmt = self.conn.prepare(
            "SELECT summary, project FROM observations WHERE candidate_id = ?1 ORDER BY id",
        )?;
        let summaries: Vec<(String, Option<String>)> = stmt
            .query_map(params![candidate_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<_, _>>()?;

        let (slug, path) =
            crate::skillfile::draft_and_write(&title, &summaries, reason, skills_dir)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A fresh, isolated (db_path, skills_dir) pair under the OS temp dir.
    /// Not cleaned up afterward (harmless clutter in /tmp) — simplest thing
    /// that avoids a tempfile dependency while staying collision-free
    /// under parallel test execution.
    fn scratch_dirs(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("myelin-test-{label}-{nanos}"));
        (root.join("myelin.db"), root.join("skills"))
    }

    fn open_test_store(label: &str) -> (Store, std::path::PathBuf) {
        let (db_path, skills_dir) = scratch_dirs(label);
        let store = Store::open(
            &db_path,
            DEFAULT_PROMOTION_REPS,
            DEFAULT_SIMILARITY_THRESHOLD,
        )
        .unwrap();
        (store, skills_dir)
    }

    fn obs(title: &str, summary: &str) -> NewObservation {
        NewObservation {
            title: title.to_string(),
            summary: summary.to_string(),
            project: None,
            context_signal: None,
            high_stakes: false,
        }
    }

    #[test]
    fn tokenize_lowercases_and_drops_short_tokens() {
        let tokens = tokenize("Run DB Migration on X");
        assert!(tokens.contains("run"));
        assert!(tokens.contains("migration"));
        assert!(!tokens.contains("db")); // len <= 2, filtered out
        assert!(!tokens.contains("x"));
    }

    #[test]
    fn jaccard_of_identical_sets_is_one() {
        let a: HashSet<String> = ["run", "migration", "service"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(jaccard(&a, &a.clone()), 1.0);
    }

    #[test]
    fn jaccard_of_disjoint_sets_is_zero() {
        let a: HashSet<String> = ["run", "migration"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["deploy", "service"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(jaccard(&a, &b), 0.0);
    }

    #[test]
    fn first_observation_creates_a_new_warming_candidate() {
        let (store, skills_dir) = open_test_store("new-candidate");
        let result = store
            .record_observation(
                obs("apply db migration hotfix", "run migrate.sh"),
                &skills_dir,
            )
            .unwrap();
        assert_eq!(result.rep_count, 1);
        assert!(!result.promoted);
        assert!(result.skill_path.is_none());
    }

    #[test]
    fn similar_observation_increments_the_same_candidate() {
        let (store, skills_dir) = open_test_store("increment-reps");
        let first = store
            .record_observation(
                obs(
                    "apply db migration hotfix",
                    "run migrate.sh then restart service",
                ),
                &skills_dir,
            )
            .unwrap();
        let second = store
            .record_observation(
                obs(
                    "apply db migration hotfix across services",
                    "run migrate.sh then restart service",
                ),
                &skills_dir,
            )
            .unwrap();
        assert_eq!(second.candidate_id, first.candidate_id);
        assert_eq!(second.rep_count, 2);
    }

    #[test]
    fn dissimilar_observation_creates_a_separate_candidate() {
        let (store, skills_dir) = open_test_store("separate-candidate");
        let first = store
            .record_observation(
                obs("apply db migration hotfix", "run migrate.sh"),
                &skills_dir,
            )
            .unwrap();
        let second = store
            .record_observation(
                obs("rotate leaked api key", "revoke and reissue the key"),
                &skills_dir,
            )
            .unwrap();
        assert_ne!(first.candidate_id, second.candidate_id);
    }

    #[test]
    fn reps_threshold_promotes_and_writes_a_real_skill_file() {
        let (store, skills_dir) = open_test_store("reps-promotion");
        let title = "apply db migration hotfix";
        let summary = "run migrate.sh then restart service then verify health";
        store
            .record_observation(obs(title, summary), &skills_dir)
            .unwrap();
        store
            .record_observation(obs(title, summary), &skills_dir)
            .unwrap();
        let third = store
            .record_observation(obs(title, summary), &skills_dir)
            .unwrap();

        assert!(third.promoted);
        let path = third.skill_path.unwrap();
        assert!(std::path::Path::new(&path).exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("name:"));
        assert!(content.contains(title));

        let skills = store.list_skills().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].promoted_reason, "reps");
        assert_eq!(skills[0].observation_count, 3);
    }

    #[test]
    fn high_stakes_promotes_on_the_first_observation() {
        let (store, skills_dir) = open_test_store("high-stakes-promotion");
        let mut input = obs("roll out CVE patch", "bump the dependency, run the scanner");
        input.high_stakes = true;
        input.context_signal = Some("security ticket: needs to land in all repos this week".into());

        let result = store.record_observation(input, &skills_dir).unwrap();
        assert!(result.promoted);
        assert_eq!(result.rep_count, 1);

        let skills = store.list_skills().unwrap();
        assert_eq!(skills[0].promoted_reason, "context_signal");
    }

    #[test]
    fn manual_promote_works_and_rejects_double_promotion() {
        let (store, skills_dir) = open_test_store("manual-promote");
        let result = store
            .record_observation(obs("one-off thing", "did it once"), &skills_dir)
            .unwrap();
        assert!(!result.promoted); // only 1 rep, no high_stakes -> still warming

        store
            .promote_candidate(result.candidate_id, &skills_dir)
            .unwrap();
        let err = store
            .promote_candidate(result.candidate_id, &skills_dir)
            .unwrap_err();
        assert!(err.to_string().contains("already promoted"));
    }

    #[test]
    fn correction_appends_to_the_live_skill_file_confirmation_does_not() {
        let (store, skills_dir) = open_test_store("feedback");
        let mut input = obs("rotate leaked api key", "revoke and reissue");
        input.high_stakes = true;
        let result = store.record_observation(input, &skills_dir).unwrap();
        let skill_id = store.list_skills().unwrap()[0].id;
        let path = result.skill_path.unwrap();

        let before = std::fs::read_to_string(&path).unwrap();
        store
            .record_skill_feedback(skill_id, "confirmation", "worked as written")
            .unwrap();
        let after_confirmation = std::fs::read_to_string(&path).unwrap();
        assert_eq!(before, after_confirmation);

        let feedback = store
            .record_skill_feedback(skill_id, "correction", "also invalidate cached tokens")
            .unwrap();
        assert_eq!(feedback.correction_count, 1);
        assert_eq!(feedback.confirmation_count, 1);
        let after_correction = std::fs::read_to_string(&path).unwrap();
        assert!(after_correction.contains("## Corrections"));
        assert!(after_correction.contains("also invalidate cached tokens"));
    }

    #[test]
    fn feedback_rejects_invalid_kind_and_unknown_skill() {
        let (store, skills_dir) = open_test_store("feedback-errors");
        let mut input = obs("rotate leaked api key", "revoke and reissue");
        input.high_stakes = true;
        store.record_observation(input, &skills_dir).unwrap();
        let skill_id = store.list_skills().unwrap()[0].id;

        let bad_kind = store
            .record_skill_feedback(skill_id, "bogus", "note")
            .unwrap_err();
        assert!(bad_kind.to_string().contains("kind must be"));

        let bad_id = store
            .record_skill_feedback(999_999, "confirmation", "note")
            .unwrap_err();
        assert!(bad_id.to_string().contains("no such skill"));
    }
}

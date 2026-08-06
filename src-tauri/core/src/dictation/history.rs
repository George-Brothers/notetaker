//! Local dictation history. This is intentionally separate from meeting
//! folders and from `notes.md`: a dictation is a text event, not a recording
//! that may be rewritten by the meeting pipeline.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const HISTORY_FILE: &str = "dictation-history.json";
const AUDIO_DIR: &str = "dictation-audio";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationEntry {
    pub id: String,
    pub created: String,
    pub text: String,
    /// Absolute path when audio retention was explicitly enabled.
    pub audio_path: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HistoryFile {
    #[serde(default)]
    entries: Vec<DictationEntry>,
}

#[derive(Debug, Clone)]
pub struct DictationHistory {
    file: PathBuf,
    audio_dir: PathBuf,
}

impl DictationHistory {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            file: data_dir.join(HISTORY_FILE),
            audio_dir: data_dir.join(AUDIO_DIR),
        }
    }

    pub fn append(&self, text: &str, samples: &[f32], keep_audio: bool) -> Result<DictationEntry> {
        let text = text.trim();
        if text.is_empty() {
            anyhow::bail!("cannot save an empty dictation")
        }

        let id = Uuid::new_v4().to_string();
        let audio_path = if keep_audio {
            fs::create_dir_all(&self.audio_dir).with_context(|| {
                format!("creating dictation audio directory {}", self.audio_dir.display())
            })?;
            let path = self.audio_dir.join(format!("{id}.wav"));
            write_wav(&path, samples)?;
            Some(path.to_string_lossy().into_owned())
        } else {
            None
        };

        let entry = DictationEntry {
            id,
            created: Utc::now().to_rfc3339(),
            text: text.to_string(),
            audio_path,
        };
        let mut history = self.read()?;
        history.entries.push(entry.clone());
        self.write(&history)?;
        Ok(entry)
    }

    pub fn last(&self) -> Result<Option<DictationEntry>> {
        Ok(self.read()?.entries.into_iter().last())
    }

    pub fn list(&self) -> Result<Vec<DictationEntry>> {
        Ok(self.read()?.entries)
    }

    fn read(&self) -> Result<HistoryFile> {
        if !self.file.exists() {
            return Ok(HistoryFile::default());
        }
        let bytes = fs::read(&self.file)
            .with_context(|| format!("reading dictation history {}", self.file.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("decoding dictation history {}", self.file.display()))
    }

    fn write(&self, history: &HistoryFile) -> Result<()> {
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating history directory {}", parent.display()))?;
        }
        let temporary = self.file.with_extension("json.tmp");
        let encoded = serde_json::to_vec_pretty(history).context("encoding dictation history")?;
        fs::write(&temporary, encoded)
            .with_context(|| format!("writing temporary history {}", temporary.display()))?;
        fs::rename(&temporary, &self.file).with_context(|| {
            format!(
                "committing dictation history {}",
                self.file.display()
            )
        })?;
        Ok(())
    }
}

fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: crate::capture::SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("creating dictation audio {}", path.display()))?;
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(value)?;
    }
    writer.finalize().context("finalizing dictation audio")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_history_is_kept_and_audio_is_off_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let history = DictationHistory::new(dir.path());
        let entry = history.append("hello world", &[0.1, -0.1], false).unwrap();
        assert_eq!(history.last().unwrap(), Some(entry.clone()));
        assert_eq!(entry.audio_path, None);
        assert!(dir.path().join(HISTORY_FILE).exists());
        assert!(!dir.path().join(AUDIO_DIR).exists());
    }

    #[test]
    fn optional_audio_is_a_lossless_local_wav_and_is_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let history = DictationHistory::new(dir.path());
        let entry = history.append("keep this", &[0.25, -0.25], true).unwrap();
        let path = PathBuf::from(entry.audio_path.unwrap());
        assert!(path.exists());
        assert!(history.last().unwrap().is_some());
    }
}

//! Meeting-summary templates, stored with the user's settings.
//!
//! A recording stores only a template id in `meta.json`. The id is stable
//! while the name, description, and headings are deliberately editable. If a
//! user deletes a template, old recordings retain that id and safely fall back
//! to the general-notes template when they are processed again.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// The template every settings file must retain. It is the safe fallback for
/// recordings whose former template was deleted.
pub const DEFAULT_ID: &str = "default";

/// The editable shape of one summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    /// Stored in recording metadata. Never change this after creation.
    pub id: String,
    pub name: String,
    pub blurb: String,
    /// Markdown headings and instructions handed to the summarizer verbatim.
    pub sections: String,
}

/// The catalog a new user starts with.
pub fn defaults() -> Vec<Template> {
    [
        (DEFAULT_ID, "General notes", "A good default for any conversation.", "## TL;DR (2-3 sentences)\n## Key points\n## Decisions\n## Action items (checkbox list, each starting with the owner's name and a colon)\n## Open questions"),
        ("one_on_one", "1:1", "A recurring check-in with one person.", "## TL;DR (2-3 sentences)\n## What they raised\n## Feedback given and received\n## Blockers\n## Action items (checkbox list, each starting with the owner's name and a colon)\n## Follow up next time"),
        ("standup", "Standup", "Short status round with a team.", "## TL;DR (1-2 sentences)\n## Updates by person\n## Blockers\n## Action items (checkbox list, each starting with the owner's name and a colon)"),
        ("lecture", "Lecture or class", "One person teaching. Optimized for studying later.", "## TL;DR (2-3 sentences)\n## Main concepts (each with a one-line explanation)\n## Definitions and formulas (verbatim where stated)\n## Examples worked through\n## Likely exam material\n## Action items (checkbox list — readings, problem sets, deadlines)"),
        ("client_call", "Client call", "An external conversation you may need to answer for.", "## TL;DR (2-3 sentences)\n## What the client asked for\n## Commitments we made (quote the wording used)\n## Commitments they made\n## Pricing, dates and numbers mentioned\n## Risks and objections\n## Action items (checkbox list, each starting with the owner's name and a colon)"),
        ("interview", "Interview", "Assessing a candidate, or being assessed.", "## TL;DR (2-3 sentences)\n## Background as described\n## Answers to each question asked\n## Strengths with evidence from the transcript\n## Concerns with evidence from the transcript\n## Action items (checkbox list, each starting with the owner's name and a colon)"),
    ]
    .into_iter()
    .map(|(id, name, blurb, sections)| Template {
        id: id.to_string(),
        name: name.to_string(),
        blurb: blurb.to_string(),
        sections: sections.to_string(),
    })
    .collect()
}

/// Finds a template, falling back to the retained general-notes template.
pub fn find<'a>(templates: &'a [Template], id: Option<&str>) -> &'a Template {
    let wanted = id.unwrap_or(DEFAULT_ID);
    templates
        .iter()
        .find(|t| t.id == wanted)
        .or_else(|| templates.iter().find(|t| t.id == DEFAULT_ID))
        .expect("validated settings always retain the default template")
}

/// Reject malformed catalogs before writing them to disk.
pub fn validate(templates: &[Template]) -> Result<()> {
    if !templates.iter().any(|t| t.id == DEFAULT_ID) {
        bail!("General notes is the fallback template and cannot be deleted");
    }
    let mut seen = std::collections::BTreeSet::new();
    for template in templates {
        if template.id.is_empty()
            || !template.id.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
        {
            bail!("template ids may use lowercase letters, numbers, and underscores only");
        }
        if !seen.insert(&template.id) {
            bail!("two templates use the id {:?}", template.id);
        }
        if template.name.trim().is_empty() || template.blurb.trim().is_empty() {
            bail!("every template needs a name and a short description");
        }
        if !template.sections.trim_start().starts_with("## ") {
            bail!("the sections for {:?} must start with a Markdown heading", template.name);
        }
        if !template.sections.to_lowercase().contains("action items") {
            bail!("the sections for {:?} must include Action items", template.name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_have_stable_ids() {
        let templates = defaults();
        validate(&templates).unwrap();
        assert_eq!(find(&templates, None).id, DEFAULT_ID);
    }

    #[test]
    fn a_deleted_template_falls_back_to_general_notes() {
        let templates = defaults();
        assert_eq!(find(&templates, Some("gone")).id, DEFAULT_ID);
    }
}

use std::path::Path;

use folio_model::{Confidence, Metadata};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataSource {
    Filename,
    LabeledFirstLines,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextMetadataGuess {
    pub title: Option<String>,
    pub author: Option<String>,
    pub source: Option<MetadataSource>,
    /// All values produced by this module are guesses, never explicit source
    /// metadata; user overrides take precedence.
    pub confidence: Option<Confidence>,
    pub evidence: Vec<String>,
}

pub fn infer_metadata(
    source_name: Option<&str>,
    text: &str,
    title_override: Option<&str>,
    author_override: Option<&str>,
) -> (Metadata, TextMetadataGuess) {
    let mut guess = source_name.and_then(filename_guess).unwrap_or_default();

    if let Some((title, author)) = labeled_first_lines(text) {
        if title.is_some() {
            guess.title = title;
            guess.source = Some(MetadataSource::LabeledFirstLines);
            guess
                .evidence
                .push("recognized a labeled title in the opening lines".to_owned());
        }
        if author.is_some() {
            guess.author = author;
            guess.source = Some(MetadataSource::LabeledFirstLines);
            guess
                .evidence
                .push("recognized a labeled author in the opening lines".to_owned());
        }
    }

    if guess.title.is_some() || guess.author.is_some() {
        guess.confidence = Some(Confidence::Heuristic);
    }

    let mut metadata = Metadata {
        title: title_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| guess.title.clone()),
        ..Metadata::default()
    };
    if let Some(author) = author_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(guess.author.as_deref())
    {
        metadata.add_author(author.to_owned());
    }
    (metadata, guess)
}

fn filename_guess(source_name: &str) -> Option<TextMetadataGuess> {
    let path = Path::new(source_name);
    let stem = path.file_stem()?.to_str()?.trim();
    if stem.is_empty() {
        return None;
    }
    let mut guess = TextMetadataGuess {
        title: Some(stem.to_owned()),
        author: None,
        source: Some(MetadataSource::Filename),
        confidence: Some(Confidence::Heuristic),
        evidence: vec!["title derived from filename stem".to_owned()],
    };

    if let Some(open) = stem.strip_prefix('《') {
        if let Some((title, suffix)) = open.split_once('》') {
            if !title.trim().is_empty() {
                guess.title = Some(title.trim().to_owned());
                if let Some(author) = suffix
                    .trim()
                    .strip_prefix("作者：")
                    .or_else(|| suffix.trim().strip_prefix("作者:"))
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    guess.author = Some(author.to_owned());
                    guess
                        .evidence
                        .push("parsed 《title》作者：author filename pattern".to_owned());
                }
            }
        }
    }

    if guess.author.is_none() {
        if let Some((title, author)) = stem.rsplit_once(" - ") {
            if !title.trim().is_empty() && !author.trim().is_empty() {
                guess.title = Some(title.trim().to_owned());
                guess.author = Some(author.trim().to_owned());
                guess
                    .evidence
                    .push("parsed title - author filename pattern".to_owned());
            }
        }
    }
    if guess.author.is_none() {
        if let Some(open) = stem.rfind('[') {
            if stem.ends_with(']') && open > 0 {
                let author = stem[open + 1..stem.len() - 1].trim();
                let title = stem[..open].trim();
                if !author.is_empty() && !title.is_empty() {
                    guess.title = Some(title.to_owned());
                    guess.author = Some(author.to_owned());
                    guess
                        .evidence
                        .push("parsed title[author] filename pattern".to_owned());
                }
            }
        }
    }
    Some(guess)
}

fn labeled_first_lines(text: &str) -> Option<(Option<String>, Option<String>)> {
    let mut title = None;
    let mut author = None;
    for line in text.lines().take(8).filter(|line| !line.trim().is_empty()) {
        let line = line.trim();
        for (prefix, is_title) in [
            ("书名：", true),
            ("书名:", true),
            ("Title:", true),
            ("标题：", true),
            ("作者：", false),
            ("作者:", false),
            ("Author:", false),
        ] {
            if let Some(value) = line
                .strip_prefix(prefix)
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                if is_title {
                    title = Some(value.to_owned());
                } else {
                    author = Some(value.to_owned());
                }
                break;
            }
        }
    }
    (title.is_some() || author.is_some()).then_some((title, author))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_documented_chinese_filename_patterns_as_heuristics() {
        for (filename, expected_title, expected_author) in [
            ("《书名》作者：作者.txt", "书名", Some("作者")),
            ("书名 - 作者.txt", "书名", Some("作者")),
            ("书名[作者].txt", "书名", Some("作者")),
        ] {
            let (metadata, guess) = infer_metadata(Some(filename), "正文", None, None);
            assert_eq!(metadata.title.as_deref(), Some(expected_title));
            assert_eq!(
                metadata.author_names().first().map(String::as_str),
                expected_author
            );
            assert_eq!(guess.confidence, Some(Confidence::Heuristic));
        }
    }

    #[test]
    fn explicit_user_overrides_win_over_guesses() {
        let (metadata, _) = infer_metadata(
            Some("Guess - Author.txt"),
            "Title: body title\nAuthor: body author",
            Some("Chosen title"),
            Some("Chosen author"),
        );
        assert_eq!(metadata.title.as_deref(), Some("Chosen title"));
        assert_eq!(metadata.author_names(), ["Chosen author"]);
    }
}

use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand, ValueEnum};
use folio_core::{
    CompressionOption, ConversionOptions, ConversionRequest, DegradationMode, Target,
};

#[derive(Debug, Parser)]
#[command(
    name = "folio",
    version,
    about = "Local multi-format book importer and EPUB/KF7/KF8/KFX converter"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Capabilities,
    Analyze {
        input: PathBuf,
        #[arg(long = "to", value_enum)]
        target: TargetArg,
        #[arg(long, value_enum, default_value_t = DegradationModeArg::Compatible)]
        mode: DegradationModeArg,
        #[arg(long)]
        linearize_complex_tables: bool,
        #[arg(long)]
        prefer_rasterization: bool,
        #[arg(long)]
        strip_embedded_fonts: bool,
        #[arg(long, value_enum, default_value_t = TextModeArg::Auto)]
        text_mode: TextModeArg,
        #[arg(long, value_enum, default_value_t = TextEncodingArg::Auto)]
        text_encoding: TextEncodingArg,
        #[arg(long, value_enum, default_value_t = ParagraphModeArg::Auto)]
        paragraph_mode: ParagraphModeArg,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        author: Option<String>,
    },
    Inspect {
        input: PathBuf,
        #[arg(long, help = "emit the full canonical semantic IR")]
        semantic: bool,
        #[arg(
            long,
            help = "emit the KFX native parser report or internal Ion sections"
        )]
        ion: bool,
        #[arg(long, help = "emit deterministic KFX resource-identity observations")]
        kfx_resource_audit: bool,
        #[arg(
            long,
            conflicts_with_all = ["semantic", "ion", "kfx_resource_audit"],
            help = "trace KFX image occurrences through native placement and Semantic IR"
        )]
        kfx_placement_audit: bool,
        #[arg(
            long,
            conflicts_with_all = ["semantic", "ion", "kfx_resource_audit", "kfx_placement_audit", "kfx_text_event_audit", "kfx_source_string_audit", "include_private_text", "include_private_strings"],
            help = "emit a content-free KFX corpus fidelity report"
        )]
        kfx_fidelity_audit: bool,
        #[arg(
            long,
            conflicts_with_all = ["semantic", "ion", "kfx_resource_audit", "kfx_placement_audit", "kfx_fidelity_audit", "kfx_text_event_audit", "kfx_source_string_audit", "include_private_text", "include_private_strings"],
            help = "emit content-free evidence from the original KFX Ion tree"
        )]
        kfx_semantic_evidence: bool,
        #[arg(
            long,
            conflicts_with_all = ["semantic", "ion", "kfx_resource_audit", "kfx_placement_audit", "kfx_fidelity_audit", "kfx_source_string_audit", "include_private_strings"],
            help = "emit content-free KFX text events with source provenance"
        )]
        kfx_text_event_audit: bool,
        #[arg(
            long,
            requires = "kfx_text_event_audit",
            help = "include private source text in this local diagnostic output"
        )]
        include_private_text: bool,
        #[arg(
            long,
            conflicts_with_all = ["semantic", "ion", "kfx_resource_audit", "kfx_placement_audit", "kfx_fidelity_audit", "kfx_text_event_audit", "include_private_text"],
            help = "emit a content-free inventory of decoded KFX Ion strings"
        )]
        kfx_source_string_audit: bool,
        #[arg(
            long,
            requires = "kfx_source_string_audit",
            help = "include private KFX source strings in this local diagnostic output"
        )]
        include_private_strings: bool,
        #[arg(
            long,
            requires = "kfx_source_string_audit",
            conflicts_with = "include_private_strings",
            value_name = "JSON_FILE",
            help = "return only inventory rows matching private strings in this temporary query file"
        )]
        source_string_query_file: Option<PathBuf>,
    },
    KfxDump {
        input: PathBuf,
        #[arg(
            long,
            help = "anonymous content-fragment alias from the KFX text-event audit, e.g. F006"
        )]
        fragment: String,
        #[arg(long, help = "numeric Ion source path from the selected TextEvent")]
        path: String,
        #[arg(
            long,
            required = true,
            help = "include bounded parent/sibling field context"
        )]
        context: bool,
        #[arg(
            long,
            help = "include private source text in this local diagnostic output"
        )]
        include_private_text: bool,
        #[arg(
            long,
            help = "include resolved Ion symbol names, which may contain source-specific identifiers"
        )]
        include_private_symbol_names: bool,
    },
    Validate {
        input: PathBuf,
    },
    Convert {
        input: PathBuf,
        #[arg(long = "to", value_enum)]
        target: TargetArg,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = CompressionArg::None)]
        compression: CompressionArg,
        #[arg(
            long,
            help = "disable deterministic IDs/order for future reference-corpus experiments"
        )]
        non_deterministic: bool,
        #[arg(long, help = "include lightweight conversion stage timings")]
        metrics: bool,
        #[arg(long, value_enum, default_value_t = DegradationModeArg::Compatible)]
        mode: DegradationModeArg,
        #[arg(long)]
        linearize_complex_tables: bool,
        #[arg(long)]
        prefer_rasterization: bool,
        #[arg(long)]
        strip_embedded_fonts: bool,
        #[arg(long, value_enum, default_value_t = TextModeArg::Auto)]
        text_mode: TextModeArg,
        #[arg(long, value_enum, default_value_t = TextEncodingArg::Auto)]
        text_encoding: TextEncodingArg,
        #[arg(long, value_enum, default_value_t = ParagraphModeArg::Auto)]
        paragraph_mode: ParagraphModeArg,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        author: Option<String>,
        #[arg(long, value_name = "JSON", help = "apply a serialized BookEditPlan")]
        edit_plan: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TargetArg {
    #[value(name = "epub")]
    Epub,
    Kf7,
    Kf8,
    Kfx,
    Kf7Kf8Combo,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DegradationModeArg {
    Strict,
    Compatible,
    Readable,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CompressionArg {
    None,
    Palmdoc,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum TextModeArg {
    #[default]
    Auto,
    Novel,
    Markdown,
    Plain,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum ParagraphModeArg {
    #[default]
    Auto,
    BlankLine,
    EveryLine,
    Indented,
    HardWrap,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum TextEncodingArg {
    #[default]
    Auto,
    #[value(name = "utf-8")]
    Utf8,
    #[value(name = "utf-16le")]
    Utf16Le,
    #[value(name = "utf-16be")]
    Utf16Be,
    #[value(name = "gb18030")]
    Gb18030,
    #[value(name = "big5")]
    Big5,
    #[value(name = "shift-jis")]
    ShiftJis,
    #[value(name = "windows-1252")]
    Windows1252,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("folio: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Capabilities => {
            println!(
                "{}",
                serde_json::to_string_pretty(&folio_core::capabilities())?
            );
        }
        Command::Analyze {
            input,
            target,
            mode,
            linearize_complex_tables,
            prefer_rasterization,
            strip_embedded_fonts,
            text_mode,
            text_encoding,
            paragraph_mode,
            title,
            author,
        } => {
            let degradation = folio_core::DegradationOptions {
                linearize_complex_tables,
                prefer_rasterization,
                strip_embedded_fonts,
            };
            let plan = folio_core::analyze_with_text_options(
                &input,
                target.into_target(),
                mode.into_mode(),
                &degradation,
                &Default::default(),
                &make_text_options(text_mode, text_encoding, paragraph_mode, title, author),
            )?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        Command::Inspect {
            input,
            semantic,
            ion,
            kfx_resource_audit,
            kfx_placement_audit,
            kfx_fidelity_audit,
            kfx_semantic_evidence,
            kfx_text_event_audit,
            include_private_text,
            kfx_source_string_audit,
            include_private_strings,
            source_string_query_file,
        } => {
            let output = if kfx_text_event_audit {
                let bytes = fs::read(&input)?;
                serde_json::to_value(folio_kfx::amazon::audit_text_events(
                    &bytes,
                    include_private_text,
                )?)?
            } else if kfx_source_string_audit {
                let bytes = fs::read(&input)?;
                if let Some(query_path) = source_string_query_file {
                    let raw_queries: Vec<serde_json::Value> =
                        serde_json::from_slice(&fs::read(query_path)?)?;
                    let queries: Vec<(String, String, bool)> = raw_queries
                        .into_iter()
                        .map(|row| {
                            let query_id = row
                                .get("query_id")
                                .and_then(serde_json::Value::as_str)
                                .ok_or("source-string query is missing a string query_id")?;
                            let text = row
                                .get("text")
                                .and_then(serde_json::Value::as_str)
                                .ok_or("source-string query is missing private text")?;
                            let match_mode = row
                                .get("match_mode")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("contains");
                            if !matches!(match_mode, "contains" | "exact") {
                                return Err(
                                    "source-string query match_mode must be contains or exact",
                                );
                            }
                            Ok((query_id.to_owned(), text.to_owned(), match_mode == "exact"))
                        })
                        .collect::<Result<_, &str>>()?;
                    folio_kfx::amazon::audit_source_string_matches(&bytes, &queries)?
                } else {
                    folio_kfx::amazon::audit_source_strings(&bytes, include_private_strings)?
                }
            } else if kfx_fidelity_audit {
                let bytes = fs::read(&input)?;
                serde_json::to_value(folio_kfx::amazon::audit_fidelity(&bytes))?
            } else if kfx_semantic_evidence {
                let bytes = fs::read(&input)?;
                serde_json::to_value(folio_kfx::amazon::audit_semantic_evidence(&bytes))?
            } else if kfx_placement_audit {
                let bytes = fs::read(&input)?;
                if !bytes.starts_with(b"CONT") {
                    return Err("--kfx-placement-audit requires an Amazon CONT KFX input".into());
                }
                serde_json::to_value(folio_kfx::amazon::audit_placements(&bytes)?)?
            } else if kfx_resource_audit {
                let bytes = fs::read(&input)?;
                if !bytes.starts_with(b"CONT") {
                    return Err("--kfx-resource-audit requires an Amazon CONT KFX input".into());
                }
                serde_json::to_value(folio_kfx::amazon::audit_resources(&bytes)?)?
            } else {
                let report = folio_core::inspect(&input)?;
                if ion {
                    let bytes = fs::read(&input)?;
                    if bytes.starts_with(b"CONT") {
                        serde_json::to_value(folio_kfx::amazon::inspect(
                            &bytes,
                            folio_kfx::amazon::ParseMode::Compatible,
                        )?)?
                    } else if bytes.starts_with(b"FFKFX\0\x01\0") {
                        serde_json::to_value(folio_kfx::inspect_compatibility_container(&bytes)?)?
                    } else {
                        return Err(
                            "--ion requires an Amazon CONT KFX or internal FFKFX input".into()
                        );
                    }
                } else if semantic {
                    report.semantic
                } else {
                    inspect_summary(&report)
                }
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        Command::KfxDump {
            input,
            fragment,
            path,
            context: _,
            include_private_text,
            include_private_symbol_names,
        } => {
            let bytes = fs::read(&input)?;
            let output = folio_kfx::amazon::audit_text_fragment_source(
                &bytes,
                &fragment,
                &path,
                include_private_text,
                include_private_symbol_names,
            )?;
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        Command::Validate { input } => {
            let report = folio_core::validate(&input)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.is_valid() {
                return Err(format!("validation failed with {} error(s)", report.errors()).into());
            }
        }
        Command::Convert {
            input,
            target,
            output,
            compression,
            non_deterministic,
            metrics,
            mode,
            linearize_complex_tables,
            prefer_rasterization,
            strip_embedded_fonts,
            text_mode,
            text_encoding,
            paragraph_mode,
            title,
            author,
            edit_plan,
        } => {
            let target = target.into_target();
            let output = output.unwrap_or_else(|| default_output(&input, target));
            let edit = match edit_plan {
                Some(path) => serde_json::from_slice(&fs::read(path)?)?,
                None => Default::default(),
            };
            let request = ConversionRequest {
                input,
                output,
                target,
                options: ConversionOptions {
                    deterministic: !non_deterministic,
                    compression: compression.into_compression(),
                    collect_metrics: metrics,
                    degradation_mode: mode.into_mode(),
                    degradation: folio_core::DegradationOptions {
                        linearize_complex_tables,
                        prefer_rasterization,
                        strip_embedded_fonts,
                    },
                    text: make_text_options(
                        text_mode,
                        text_encoding,
                        paragraph_mode,
                        title,
                        author,
                    ),
                },
                edit,
            };
            let report = folio_core::convert_with_progress(
                &request,
                &folio_core::CancellationToken::new(),
                |event| {
                    let fraction = event
                        .fraction
                        .map(|value| format!(" {:.0}%", value * 100.0))
                        .unwrap_or_default();
                    eprintln!("{:?}{}: {}", event.stage, fraction, event.message);
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }
    Ok(())
}

fn make_text_options(
    mode: TextModeArg,
    encoding: TextEncodingArg,
    paragraph_mode: ParagraphModeArg,
    title: Option<String>,
    author: Option<String>,
) -> folio_text::TextImportOptions {
    use folio_text::{ParagraphMode, TextEncoding, TextImportMode, TextImportOptions};
    TextImportOptions {
        mode: match mode {
            TextModeArg::Auto => TextImportMode::Auto,
            TextModeArg::Novel => TextImportMode::Novel,
            TextModeArg::Markdown => TextImportMode::Markdown,
            TextModeArg::Plain => TextImportMode::Plain,
        },
        paragraph_mode: match paragraph_mode {
            ParagraphModeArg::Auto => ParagraphMode::Auto,
            ParagraphModeArg::BlankLine => ParagraphMode::BlankLine,
            ParagraphModeArg::EveryLine => ParagraphMode::EveryLine,
            ParagraphModeArg::Indented => ParagraphMode::Indented,
            ParagraphModeArg::HardWrap => ParagraphMode::HardWrap,
        },
        encoding_override: match encoding {
            TextEncodingArg::Auto => None,
            TextEncodingArg::Utf8 => Some(TextEncoding::Utf8),
            TextEncodingArg::Utf16Le => Some(TextEncoding::Utf16Le),
            TextEncodingArg::Utf16Be => Some(TextEncoding::Utf16Be),
            TextEncodingArg::Gb18030 => Some(TextEncoding::Gb18030),
            TextEncodingArg::Big5 => Some(TextEncoding::Big5),
            TextEncodingArg::ShiftJis => Some(TextEncoding::ShiftJis),
            TextEncodingArg::Windows1252 => Some(TextEncoding::Windows1252),
        },
        title_override: title,
        author_override: author,
    }
}

impl TargetArg {
    fn into_target(self) -> Target {
        match self {
            Self::Epub => Target::EPUB,
            Self::Kf7 => Target::KF7,
            Self::Kf8 => Target::KF8,
            Self::Kfx => Target::KFX,
            Self::Kf7Kf8Combo => Target::KF7KF8Combo,
        }
    }
}

impl DegradationModeArg {
    fn into_mode(self) -> DegradationMode {
        match self {
            Self::Strict => DegradationMode::Strict,
            Self::Compatible => DegradationMode::Compatible,
            Self::Readable => DegradationMode::Readable,
        }
    }
}

impl CompressionArg {
    fn into_compression(self) -> CompressionOption {
        match self {
            Self::None => CompressionOption::None,
            Self::Palmdoc => CompressionOption::PalmDoc,
        }
    }
}

fn default_output(input: &Path, target: Target) -> PathBuf {
    let mut output = input.with_extension(target.extension());
    if output == input {
        output.set_file_name(format!(
            "{}-converted.{}",
            input
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("book"),
            target.extension()
        ));
    }
    output
}

fn inspect_summary(report: &folio_core::InspectReport) -> serde_json::Value {
    serde_json::json!({
        "format": report.format,
        "input_report": report.input_report,
        "semantic_report": report.semantic_report,
        "diagnostics": report.diagnostics,
    })
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../tests/unit/crates/folio-cli/src/main.rs"]
mod tests;

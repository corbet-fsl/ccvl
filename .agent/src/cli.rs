use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};

use crate::check;
use crate::downstream;
use crate::format;
use crate::measure;
use crate::opportunity;
use crate::public;
use crate::render::{self, Compiler};
use crate::skills;
use crate::stations;
use crate::workspace::Workspace;

const COMPARE_HELP: &str = "\
Items:
  harvard/d-plus            style/substyle, rendered fresh (default locale en-ch)
  harvard/compact@de-ch:2   with a per-item locale and page preset
  default, cluster/default  the workspace default style, or a style's first substyle
  opportunities/acme/lead   an application record or variant directory, in its own language
  old/cv-4.pdf              an existing PDF, used as is

Examples:
  ccvl compare harvard/d-plus cluster/d-plus
  ccvl compare harvard/d-plus harvard/compact harvard/standard --pages 4 --layout sheet
  ccvl compare harvard/d-plus@de-ch harvard/d-plus@en-ch --only 1
  ccvl compare --doc cl harvard/frame harvard/left-rule --png --json

Output: .agent/cache/comparisons/<name>.pdf (rendered variants and previews in
.agent/cache/comparisons/<name>/). Interleaved pages run 1A 1B ... 1N, 2A ...;
see .agent/docs/compare.md.";

#[derive(Debug, Parser)]
#[command(
    name = "ccvl",
    version,
    about = "Deterministic CV and cover-letter compiler"
)]
struct Args {
    #[arg(long, global = true, value_name = "DIRECTORY")]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Export a neutral style variant for native or web consumers.
    ExportStyle {
        #[arg(value_parser = ["cv", "cl"])]
        document: String,
        locale: String,
        #[arg(long)]
        style: Option<String>,
        #[arg(long)]
        substyle: Option<String>,
        #[arg(long)]
        pages: Option<usize>,
        #[arg(long)]
        paper: Option<String>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Render a versioned style bundle with explicit user inputs.
    RenderStyle {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        application: PathBuf,
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Prepare and validate an independently reviewed document package.
    Review {
        #[command(subcommand)]
        command: crate::review::Command,
    },
    /// Print the compiled runtime source identity without opening a workspace.
    RuntimeId,
    /// Maintainer: source-bound release evidence for native binaries.
    #[command(hide = true)]
    ReleaseEvidence {
        #[command(subcommand)]
        command: crate::release_evidence::Command,
    },
    /// Maintainer: check a private downstream on Crow with its published runtime.
    #[command(hide = true)]
    PrivateCheck,
    /// Verify the self-contained binary and workspace.
    Setup,
    /// Show what the dependency-free setup requires.
    Bootstrap,
    /// Report the embedded runtime and workspace.
    Doctor,
    /// Run every deterministic workspace and document check.
    Check {
        /// Limit document rendering and verification to one `<cv|cl>/<style>`;
        /// repeat for several. Workspace-wide checks always run.
        #[arg(long = "style", value_name = "DOC/STYLE")]
        styles: Vec<String>,
        /// Also hint at figures in application wording that no interview
        /// fact contains (opt-in; rephrased figures can differ).
        #[arg(long)]
        figures: bool,
    },
    /// Check station coverage and MECE ownership.
    ProfileStatus {
        #[arg(default_value = "interview/stations.toml")]
        plan: PathBuf,
        #[arg(long)]
        verify_sources: bool,
        /// Print the whole interview as Markdown for a quick review.
        #[arg(long)]
        list: bool,
    },
    /// Measure the CVL templates.
    Measure {
        #[arg(long)]
        all: bool,
        /// Limit measurement to one `<cv|cl>/<style>`; repeat for several.
        #[arg(long = "style", value_name = "DOC/STYLE")]
        styles: Vec<String>,
    },
    /// Measure one keyed opportunity.
    MeasureOpportunity {
        organisation_key: String,
        position_key: String,
        #[arg(long)]
        all: bool,
    },
    /// Run all checks required before publishing.
    PublicCheck {
        /// Retain freshly verified PDFs in a new directory.
        #[arg(long)]
        artifacts: Option<PathBuf>,
        /// Limit document rendering and verification to one `<cv|cl>/<style>`;
        /// repeat for several. Workspace-wide checks always run.
        #[arg(long = "style", value_name = "DOC/STYLE")]
        styles: Vec<String>,
    },
    /// Verify that a private downstream differs only in explicitly owned paths.
    DownstreamCheck {
        #[arg(long, default_value = "ccvl-downstream.json")]
        policy: PathBuf,
        #[arg(long)]
        upstream_ref: Option<String>,
    },
    /// Build every CVL template and page preset.
    Build,
    /// List every registered document variant and its output as JSON.
    ListDocuments,
    /// List each document's styles, substyles, empty slots and defaults as JSON.
    ListStyles,
    /// Explain merged adapter settings and the source of each value as JSON.
    ExplainStyle {
        #[arg(value_parser = ["cv", "cl"])]
        document: String,
        locale: String,
        #[arg(long)]
        style: Option<String>,
        #[arg(long)]
        substyle: Option<String>,
        /// Paper preset declared by the selected style.
        #[arg(long)]
        paper: Option<String>,
    },
    /// Create one keyed opportunity without overwriting an existing record.
    NewOpportunity {
        organisation_key: String,
        position_key: String,
        /// Skip the cover letter: writes `generate_cl = false` and no `[cl]`
        /// table, so there is nothing to delete afterwards.
        #[arg(long)]
        no_cover_letter: bool,
    },
    /// Build one CV.
    BuildCv {
        locale: String,
        pages: Option<usize>,
        /// CV style, or `default`. Defaults to the record's selection, then to
        /// the workspace default; `--substyle default` selects its first substyle.
        #[arg(long)]
        style: Option<String>,
        #[arg(long)]
        substyle: Option<String>,
        #[arg(long)]
        application: Option<PathBuf>,
        #[arg(long)]
        profile: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        /// Paper preset declared by the selected style.
        #[arg(long)]
        paper: Option<String>,
    },
    /// Build one cover letter.
    BuildCl {
        locale: String,
        #[arg(long)]
        pages: Option<usize>,
        /// Cover-letter style, or `default`. Defaults to the record's selection,
        /// then to the workspace default; `--substyle default` selects its first
        /// substyle.
        #[arg(long)]
        style: Option<String>,
        #[arg(long)]
        substyle: Option<String>,
        #[arg(long)]
        application: Option<PathBuf>,
        #[arg(long)]
        profile: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        /// Paper preset declared by the selected style.
        #[arg(long)]
        paper: Option<String>,
    },
    /// Render n variants into one side-by-side comparison PDF.
    #[command(after_help = COMPARE_HELP)]
    Compare {
        /// Two or more variants: `<style>/<substyle>[@<locale>][:<preset>]`,
        /// `default`, an application.toml, a variant directory or a PDF.
        #[arg(required = true, num_args = 2.., value_name = "ITEM")]
        items: Vec<String>,
        /// Document to render for style and record items.
        #[arg(long = "doc", value_parser = ["cv", "cl"], default_value = "cv")]
        document: String,
        /// Locale for style items without `@<locale>`; defaults to en-ch when
        /// the style declares it, otherwise to its first locale.
        #[arg(long)]
        locale: Option<String>,
        /// Page preset for rendered items without `:<preset>`.
        #[arg(long)]
        pages: Option<usize>,
        /// Show only these page rows, such as `1-2` or `1,3`.
        #[arg(long, value_name = "PAGES")]
        only: Option<String>,
        /// `interleaved` pages for an n-column viewer, or one wide `sheet`
        /// page per row that works in every viewer.
        #[arg(long, value_parser = ["interleaved", "sheet"], default_value = "interleaved")]
        layout: String,
        /// Output name under .agent/cache/comparisons/; derived from the
        /// items when omitted.
        #[arg(long)]
        name: Option<String>,
        /// Also write one PNG sheet per row for inspection.
        #[arg(long)]
        png: bool,
        /// Show line measurements of each rendered variant side by side.
        #[arg(long)]
        metrics: bool,
        /// Print the summary as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Build one keyed opportunity package.
    BuildOpportunity {
        organisation_key: String,
        position_key: String,
    },
    /// Rebuild one CV whenever its inputs change.
    WatchCv {
        locale: String,
        pages: Option<usize>,
        #[arg(long)]
        style: Option<String>,
        #[arg(long)]
        substyle: Option<String>,
        /// Paper preset declared by the selected style.
        #[arg(long)]
        paper: Option<String>,
    },
    /// Rebuild one cover letter whenever its inputs change.
    WatchCl {
        locale: String,
        #[arg(long)]
        pages: Option<usize>,
        #[arg(long)]
        style: Option<String>,
        #[arg(long)]
        substyle: Option<String>,
        /// Paper preset declared by the selected style.
        #[arg(long)]
        paper: Option<String>,
    },
    /// Rebuild one keyed opportunity (PDFs plus resolved .typ copies)
    /// whenever its template, record, or generated typst outputs change.
    WatchOpportunity {
        organisation_key: String,
        position_key: String,
    },
    /// Format Typst sources with the embedded formatter.
    Fmt {
        #[arg(long)]
        check: bool,
    },
    /// Run the strict small-model skill evaluation.
    SkillEval {
        #[arg(long, default_value = ".agent/tests/skill-cases.json")]
        cases: PathBuf,
        #[arg(long, default_value = ".agent/skills")]
        skills_root: PathBuf,
        #[arg(long, default_value = ".agent/cache/skill-eval/report.json")]
        output: PathBuf,
        /// Score complete option assessments without contacting a model provider.
        #[arg(long)]
        response_file: Option<PathBuf>,
        #[arg(long)]
        summary: Option<PathBuf>,
        /// Groq request model, or a caller-provided label for a saved response.
        #[arg(long)]
        model: Option<String>,
    },
}

pub fn run() -> Result<ExitCode> {
    let args = Args::parse();
    if matches!(args.command, Command::RuntimeId) {
        println!("{}", crate::runtime::ID);
        return Ok(ExitCode::SUCCESS);
    }
    // Maintainer routes run before any workspace exists for the target.
    if let Command::ReleaseEvidence { command } = &args.command {
        return Ok(crate::release_evidence::run(args.root.as_deref(), command));
    }
    if matches!(args.command, Command::PrivateCheck) {
        return Ok(crate::private_check::run(args.root.as_deref()));
    }
    if let Command::RenderStyle {
        bundle,
        application,
        profile,
        output,
    } = &args.command
    {
        crate::bundle::render(bundle, application, profile, output)?;
        return Ok(ExitCode::SUCCESS);
    }
    let workspace = Workspace::discover(args.root.as_deref())?;
    crate::runtime::verify(workspace.root())?;
    let mut exit_code = ExitCode::SUCCESS;
    match args.command {
        Command::ExportStyle {
            document,
            locale,
            style,
            substyle,
            pages,
            paper,
            output,
        } => {
            let bundle = crate::bundle::export(
                &workspace,
                &document,
                &locale,
                style.as_deref(),
                substyle.as_deref(),
                pages,
                paper.as_deref(),
            )?;
            std::fs::write(&output, serde_json::to_vec_pretty(&bundle)?)?;
            println!("{} {} {}", bundle.id, bundle.version, output.display());
        }
        Command::RenderStyle { .. } => unreachable!("handled without a workspace"),
        Command::Review { command } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&crate::review::run(&workspace, command)?)?
            );
        }
        Command::RuntimeId | Command::ReleaseEvidence { .. } | Command::PrivateCheck => {
            unreachable!("handled before workspace discovery")
        }
        Command::Setup => {
            doctor(&workspace)?;
            check::run(&workspace)?;
            println!("ccvl is ready. No external runtime or fonts are required.");
        }
        Command::Bootstrap => {
            println!("ccvl bootstrap plan");
            println!(
                "platform: {}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            println!("runtime dependencies: none");
            println!("embedded: Typst 0.15.1 | Typstyle 0.15.1 | 16 font files");
        }
        Command::Doctor => doctor(&workspace)?,
        Command::Check { styles, figures } => {
            let styles = crate::styles::StyleFilter::parse(&workspace, &styles)?;
            if figures {
                crate::lint::figure_hints(&workspace)?
                    .iter()
                    .for_each(|hint| println!("WARN {hint}"));
            }
            check::run_selected(&workspace, None, &styles)?;
            println!(
                "All data, station, source, skill, font, reproducibility, CV, and cover-letter checks passed{}.",
                document_scope(&styles)
            );
        }
        Command::ProfileStatus {
            plan,
            verify_sources,
            list,
        } => stations::profile_status(&workspace, &plan, verify_sources, list)?,
        Command::Measure { all, styles } => {
            let styles = crate::styles::StyleFilter::parse(&workspace, &styles)?;
            let failures = measure::measure(
                &workspace,
                &measure::cvl_specs_for(&workspace, &styles)?,
                all,
                true,
            )?;
            ensure!(
                failures.is_empty(),
                "{} line-contract failure(s)",
                failures.len()
            );
        }
        Command::MeasureOpportunity {
            organisation_key,
            position_key,
            all,
        } => {
            let specs = measure::keyed_specs(&workspace, &organisation_key, &position_key)?;
            let failures = measure::measure(&workspace, &specs, all, true)?;
            ensure!(
                failures.is_empty(),
                "{} line-contract failure(s)",
                failures.len()
            );
        }
        Command::PublicCheck { artifacts, styles } => {
            let styles = crate::styles::StyleFilter::parse(&workspace, &styles)?;
            check::run_selected(&workspace, artifacts.as_deref(), &styles)?;
            public::validate_boundary(&workspace)?;
            println!(
                "Public-boundary checks passed{}. Review .agent/docs/public-identifiers.md before publishing.",
                document_scope(&styles)
            );
        }
        Command::DownstreamCheck {
            policy,
            upstream_ref,
        } => downstream::validate(&workspace, &policy, upstream_ref.as_deref())?,
        Command::Build => print_outputs(render::render_cvl(&workspace)?),
        Command::ListDocuments => println!(
            "{}",
            serde_json::to_string_pretty(&render::list_documents(&workspace)?)?
        ),
        Command::ListStyles => println!(
            "{}",
            serde_json::to_string_pretty(&crate::styles::list_styles(&workspace)?)?
        ),
        Command::ExplainStyle {
            document,
            locale,
            style,
            substyle,
            paper,
        } => {
            let document = if document == "cv" { "cv" } else { "cl" };
            let selected = crate::styles::selection(
                &workspace,
                document,
                style.as_deref(),
                substyle.as_deref(),
            )?;
            let leaf = crate::styles::leaf(&workspace, document, &locale, &selected)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&crate::settings::explain(
                    &workspace,
                    &leaf,
                    paper.as_deref()
                )?)?
            );
        }
        Command::NewOpportunity {
            organisation_key,
            position_key,
            no_cover_letter,
        } => {
            let path = opportunity::create_record(
                &workspace,
                &organisation_key,
                &position_key,
                !no_cover_letter,
            )?;
            println!("Created {}", workspace.relative(&path)?.display());
        }
        Command::BuildCv {
            locale,
            pages,
            style,
            substyle,
            application,
            profile,
            output,
            paper,
        } => print_outputs(vec![build_document(
            &workspace,
            "cv",
            &locale,
            pages,
            style.as_deref(),
            substyle.as_deref(),
            application.as_deref(),
            profile.as_deref(),
            output.as_deref(),
            paper.as_deref(),
        )?]),
        Command::BuildCl {
            locale,
            pages,
            style,
            substyle,
            application,
            profile,
            output,
            paper,
        } => print_outputs(vec![build_document(
            &workspace,
            "cl",
            &locale,
            pages,
            style.as_deref(),
            substyle.as_deref(),
            application.as_deref(),
            profile.as_deref(),
            output.as_deref(),
            paper.as_deref(),
        )?]),
        Command::Compare {
            items,
            document,
            locale,
            pages,
            only,
            layout,
            name,
            png,
            metrics,
            json,
        } => {
            let outcome = crate::compare::run(
                &workspace,
                &crate::compare::Request {
                    document: if document == "cv" { "cv" } else { "cl" },
                    items,
                    locale,
                    pages,
                    only,
                    layout: crate::compare::Layout::parse(&layout)?,
                    name,
                    png,
                    metrics,
                },
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            } else {
                println!("{outcome}");
            }
        }
        Command::BuildOpportunity {
            organisation_key,
            position_key,
        } => print_outputs(render::render_opportunity(
            &workspace,
            &organisation_key,
            &position_key,
        )?),
        Command::WatchCv {
            locale,
            pages,
            style,
            substyle,
            paper,
        } => {
            watch_cv(
                &workspace,
                &locale,
                pages,
                style.as_deref(),
                substyle.as_deref(),
                paper.as_deref(),
            )?;
        }
        Command::WatchCl {
            locale,
            pages,
            style,
            substyle,
            paper,
        } => {
            watch_cl(
                &workspace,
                &locale,
                pages,
                style.as_deref(),
                substyle.as_deref(),
                paper.as_deref(),
            )?;
        }
        Command::WatchOpportunity {
            organisation_key,
            position_key,
        } => watch_opportunity(&workspace, &organisation_key, &position_key)?,
        Command::Fmt { check } => format::format_typst(&workspace, check)?,
        Command::SkillEval {
            cases,
            skills_root,
            output,
            response_file,
            summary,
            model,
        } => {
            let cases = resolve(&workspace, &cases);
            let skills_root = resolve(&workspace, &skills_root);
            let output = resolve(&workspace, &output);
            let response_file = response_file
                .as_deref()
                .map(|path| resolve(&workspace, path));
            let summary = summary.as_deref().map(|path| resolve(&workspace, path));
            let model = model.or_else(|| {
                response_file
                    .is_none()
                    .then(|| std::env::var("GROQ_MODEL").ok())
                    .flatten()
            });
            let outcome = skills::run_hosted_evaluation(
                &workspace,
                &cases,
                &skills_root,
                &output,
                response_file.as_deref(),
                model.as_deref(),
                summary.as_deref(),
            )?;
            println!(
                "Skill evaluation report: {} ({})",
                output.display(),
                outcome.status()
            );
            exit_code = ExitCode::from(outcome.exit_code());
        }
    }
    Ok(exit_code)
}

/// Name a limited document scope so a filtered pass never reads as complete.
fn document_scope(styles: &crate::styles::StyleFilter) -> String {
    if styles.is_all() {
        String::new()
    } else {
        format!(" (documents limited to {styles})")
    }
}

fn doctor(workspace: &Workspace) -> Result<()> {
    ensure!(
        ctypst::fonts::documents().len() == 16,
        "embedded font set is incomplete"
    );
    println!("ccvl {}", env!("CARGO_PKG_VERSION"));
    println!(
        "platform: {}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("workspace: {}", workspace.root().display());
    println!("Typst engine: embedded 0.15.1");
    println!("Typstyle formatter: embedded 0.15.1");
    println!("fonts: 16 embedded files; system fonts disabled");
    println!("external runtime dependencies: none");
    Ok(())
}

/// Render one `build-cv` or `build-cl` request through the shared build path.
#[allow(clippy::too_many_arguments)]
fn build_document(
    workspace: &Workspace,
    document: &'static str,
    locale: &str,
    pages: Option<usize>,
    style: Option<&str>,
    substyle: Option<&str>,
    application: Option<&Path>,
    profile: Option<&Path>,
    output: Option<&Path>,
    paper: Option<&str>,
) -> Result<PathBuf> {
    let selection = cli_selection(workspace, style, substyle, application, document)?;
    let leaf = crate::styles::leaf(workspace, document, locale, &selection)?;
    let spec = render::build_spec(workspace, &leaf, application, pages, profile, output, paper)?;
    Compiler::new(workspace)?.render(workspace, &spec)
}

/// Explicit `--style`/`--substyle` win; otherwise the record at `application`
/// selects; otherwise the workspace default style and its first substyle render.
fn cli_selection(
    workspace: &Workspace,
    style: Option<&str>,
    substyle: Option<&str>,
    application: Option<&Path>,
    document: &str,
) -> Result<crate::styles::Selection> {
    let record = application
        .map(|path| {
            let path = workspace.existing_inside(path)?;
            let location = workspace.relative(&path)?.display().to_string();
            crate::styles::record_selection(
                workspace,
                document,
                &crate::content::read_record(workspace, &path)?,
                &location,
            )
        })
        .transpose()?;
    crate::styles::selection(
        workspace,
        document,
        style.or(record.as_ref().map(|selection| selection.style.as_str())),
        substyle.or(record.as_ref().map(|selection| selection.substyle.as_str())),
    )
}

fn watch_cv(
    workspace: &Workspace,
    locale: &str,
    pages: Option<usize>,
    style: Option<&str>,
    substyle: Option<&str>,
    paper: Option<&str>,
) -> Result<()> {
    crate::watch::run(workspace, &format!("{locale} CV"), |workspace| {
        let selection = crate::styles::selection(workspace, "cv", style, substyle)?;
        let leaf = render::cv_leaf(workspace, locale, &selection)?;
        let spec = render::cvl_spec_with_paper(
            workspace,
            &leaf,
            pages.unwrap_or(leaf.default_pages),
            paper,
        )?;
        Ok(vec![Compiler::new(workspace)?.render(workspace, &spec)?])
    })
}

fn watch_cl(
    workspace: &Workspace,
    locale: &str,
    pages: Option<usize>,
    style: Option<&str>,
    substyle: Option<&str>,
    paper: Option<&str>,
) -> Result<()> {
    crate::watch::run(workspace, &format!("{locale} cover letter"), |workspace| {
        let selection = crate::styles::selection(workspace, "cl", style, substyle)?;
        let leaf = render::cl_leaf(workspace, locale, &selection)?;
        let spec = render::cvl_spec_with_paper(
            workspace,
            &leaf,
            pages.unwrap_or(leaf.default_pages),
            paper,
        )?;
        Ok(vec![Compiler::new(workspace)?.render(workspace, &spec)?])
    })
}

fn watch_opportunity(workspace: &Workspace, organisation: &str, position: &str) -> Result<()> {
    // Reject malformed keys, while allowing a temporarily missing record to be
    // restored without restarting the watcher.
    opportunity::record_path(workspace, organisation, position, false)?;
    crate::watch::run(
        workspace,
        &format!("opportunity {organisation}/{position}"),
        |workspace| render::render_opportunity(workspace, organisation, position),
    )
}

fn print_outputs(outputs: Vec<PathBuf>) {
    for output in outputs {
        println!("Rendered {}", output.display());
    }
}

fn resolve(workspace: &Workspace, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.path(path)
    }
}

#[cfg(test)]
mod tests;

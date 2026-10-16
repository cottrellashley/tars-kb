use anyhow::{Context, bail, ensure};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
};
use tars_kb::{
    Result,
    library::{self, Base},
    memory::{self, FactFilter},
    output, pages,
    search::{self, SearchOptions},
    storage, temporal,
};

#[derive(Parser)]
#[command(
    version,
    about = "Fast local knowledge bases and temporal memory",
    long_about = "Portable Markdown/JSON knowledge with scoped discovery, safe writes, temporal assertions and rebuildable search. No network access or model calls."
)]
struct Cli {
    #[arg(
        long,
        global = true,
        help = "Library root (or TARS_KB_ROOT; default ./knowledge-bases)"
    )]
    root: Option<PathBuf>,
    #[arg(long, global = true, help = "Machine-readable JSON on stdout")]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}
#[derive(Args, Clone)]
struct Bounds {
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long, default_value_t = 8192)]
    max_bytes: usize,
    #[arg(long)]
    cursor: Option<String>,
}
#[derive(ValueEnum, Clone, Copy, PartialEq)]
enum Mode {
    Literal,
    Ranked,
}
#[derive(Subcommand)]
enum Commands {
    /// Show supported contracts and operations without opening a library.
    Capabilities,
    /// List base metadata only; legacy JSON is an array.
    List {
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        max_bytes: Option<usize>,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Create an independent base and register it atomically.
    Init {
        base: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "Owner")]
        owner: String,
    },
    /// Read one page; default output preserves the v1 reference interface.
    Show {
        base: String,
        page: Option<String>,
        #[arg(long)]
        section: Option<String>,
        #[arg(long)]
        with_revision: bool,
    },
    /// Bounded page read with a revision for subsequent checked edits.
    Read {
        base: String,
        page: Option<String>,
        #[arg(long)]
        section: Option<String>,
        #[arg(long, default_value_t = 8192)]
        max_bytes: usize,
    },
    /// Discover page headings without returning the page body.
    Outline {
        base: String,
        page: String,
        #[command(flatten)]
        bounds: Bounds,
    },
    /// Literal v1 search by default; ranked mode queries a scoped FTS index.
    Search {
        base: String,
        query: String,
        #[arg(long, value_enum, default_value = "literal")]
        mode: Mode,
        #[arg(long)]
        verified: bool,
        #[arg(long)]
        concept: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        max_bytes: Option<usize>,
    },
    /// Rebuild or incrementally update one disposable index.
    Index {
        base: String,
        #[arg(long)]
        rebuild: bool,
    },
    /// Pack bounded existing evidence for a task; no generated answer.
    Context {
        base: String,
        #[arg(long)]
        query: String,
        #[arg(long)]
        concept: Option<String>,
        #[arg(long, value_enum, default_value = "literal")]
        mode: Mode,
        #[arg(long)]
        verified: bool,
        #[arg(long, default_value_t = 8)]
        limit: usize,
        #[arg(long, default_value_t = 8192)]
        max_bytes: usize,
    },
    /// Validate all selected files, references and supported memory records.
    Validate { base: Option<String> },
    /// Export one coherent partition into a new directory.
    Export {
        base: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long,default_value="markdown",value_parser=["markdown","html","json"])]
        format: String,
    },
    #[command(subcommand)]
    Page(PageCommand),
    /// Open a private draft in EDITOR, then save with a revision check.
    Edit { base: String, page: String },
    #[command(subcommand)]
    Source(SourceCommand),
    #[command(subcommand)]
    Memory(MemoryCommand),
    /// Discover concepts without reading assertion bodies.
    Concepts {
        base: String,
        #[command(flatten)]
        bounds: Bounds,
    },
    #[command(subcommand)]
    Concept(ConceptCommand),
    /// Add a sourced assertion using structured input.
    Remember {
        base: String,
        concept: Option<String>,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        idempotency_key: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Correct, change, reconfirm or retract an assertion without losing history.
    Revise {
        base: String,
        id: String,
        #[arg(long,value_parser=["correct","change","reconfirm","retract"])]
        mode: String,
        #[arg(long)]
        expect_revision: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        idempotency_key: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Read one complete assertion and optional source evidence.
    Fact {
        base: String,
        id: String,
        #[arg(long)]
        evidence: bool,
        #[arg(long, default_value_t = 8192)]
        max_bytes: usize,
    },
    /// Query claims by concept, valid time and knowledge time.
    Facts {
        base: String,
        concept: Option<String>,
        #[arg(long)]
        valid_at: Option<String>,
        #[arg(long)]
        known_at: Option<String>,
        #[arg(long)]
        known_revision: Option<u64>,
        #[arg(long,value_parser=["all","applicable","last-observed"])]
        view: Option<String>,
        #[command(flatten)]
        bounds: Bounds,
    },
    /// Read retained revisions of one assertion.
    History {
        base: String,
        id: String,
        #[command(flatten)]
        bounds: Bounds,
    },
    /// Atomically apply memory operations in one base; supports retry receipts.
    Batch {
        base: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        idempotency_key: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Recover an interrupted cooperative write (also done on ordinary open).
    Recover { base: String },
    #[command(subcommand)]
    Spec(SpecCommand),
    #[command(subcommand)]
    Schema(SchemaCommand),
    #[command(subcommand)]
    Time(TimeCommand),
}
#[derive(Subcommand)]
enum PageCommand {
    Create {
        base: String,
        id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        input: PathBuf,
    },
    #[command(alias = "write")]
    Replace {
        base: String,
        id: String,
        #[arg(long)]
        expect_revision: String,
        #[arg(long)]
        input: PathBuf,
    },
    Patch {
        base: String,
        id: String,
        #[arg(long)]
        expect_revision: String,
        #[arg(long)]
        input: PathBuf,
    },
}
#[derive(Subcommand)]
enum SourceCommand {
    Add {
        base: String,
        #[arg(long)]
        input: PathBuf,
    },
}
#[derive(Subcommand)]
enum MemoryCommand {
    Init {
        base: String,
        #[arg(long, default_value = "tars-memory/1.0")]
        profile: String,
    },
}
#[derive(Subcommand)]
enum ConceptCommand {
    Init {
        base: String,
        id: String,
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        summary: Option<String>,
    },
    Get {
        base: String,
        id: String,
        #[arg(long, default_value_t = 8192)]
        max_bytes: usize,
    },
}
#[derive(Subcommand)]
enum SpecCommand {
    List,
    Show {
        name: String,
        #[arg(long)]
        section: Option<String>,
    },
}
#[derive(Subcommand)]
enum SchemaCommand {
    List,
    Show { name: String },
}
#[derive(Subcommand)]
enum TimeCommand {
    Normalize {
        value: String,
    },
    Resolve {
        value: String,
        #[arg(long)]
        zone: Option<String>,
        #[arg(long)]
        reference: Option<String>,
    },
}

fn input_bytes(path: &Path) -> Result<Vec<u8>> {
    if path == Path::new("-") {
        let mut b = Vec::new();
        io::stdin()
            .take(storage::MAX_FILE + 1)
            .read_to_end(&mut b)?;
        ensure!(
            b.len() as u64 <= storage::MAX_FILE,
            "size_limit: stdin exceeds 64 MiB"
        );
        Ok(b)
    } else {
        storage::read_bytes(path)
    }
}
fn input(path: &Path) -> Result<Value> {
    storage::parse_json(&input_bytes(path)?)
}
fn text_input(path: &Path) -> Result<String> {
    Ok(String::from_utf8(input_bytes(path)?)?)
}
fn root(cli: &Cli) -> Result<PathBuf> {
    Ok(cli
        .root
        .clone()
        .or_else(|| std::env::var_os("TARS_KB_ROOT").map(PathBuf::from))
        .unwrap_or(std::env::current_dir()?.join("knowledge-bases")))
}
fn paged(base: &Base, value: Value, key: &str, b: &Bounds, scope: &str) -> Result<Value> {
    let revision = if let Some(s) = value.get("current_sequence").or(value.get("sequence")) {
        format!("{}:{s}", base.revision()?)
    } else {
        storage::hash(&serde_json::to_vec(&value)?)
    };
    output::bounded(
        value,
        key,
        b.limit,
        b.max_bytes,
        b.cursor.as_deref(),
        &format!("{}:{}:{scope}", base.library.display(), base.id),
        &revision,
    )
}
fn capabilities() -> Value {
    json!({"output_version":"1.0","binary":{"name":"tars-kb","version":env!("CARGO_PKG_VERSION")},"storage":{"read":["1.x"],"write":["1.x"],"unknown_required_features":"reject"},"memory":{"read":[memory::PROFILE],"write":[memory::PROFILE]},"output_protocols":["legacy-v1","1.0"],"index_schema":"1","network_access":false,"daemon_required":false,"temporal_kinds":["timeless","observed","event","interval","unknown"],"commands":["list","init","show","read","outline","search","index","context","validate","export","page","edit","source","memory","concepts","concept","remember","revise","fact","facts","history","batch","recover","spec","schema","time"],"fault_injection":cfg!(feature="fault-injection")})
}
fn dry_run(base: &Base, ops: Vec<Value>, idempotency: Option<&str>) -> Result<Value> {
    let temp = tempfile::tempdir()?;
    let library = temp.path().join("library");
    storage::private_dir(&library)?;
    let dest = library.join(format!("bases/{}", base.id));
    storage::private_dir(&dest)?;
    for path in library::files_recursive(&base.root)? {
        storage::atomic(&dest.join(&path), &base.read(&path)?)?;
    }
    storage::atomic(
        &library.join("catalog.json"),
        &storage::json_bytes(
            &json!({"schema_version":"1.0","knowledge_bases":[{"id":base.id,"path":format!("bases/{}",base.id)}]}),
        )?,
    )?;
    let mut copy = Base::open(&library, &base.id)?;
    let mut result = memory::batch(&mut copy, ops, idempotency)?;
    result["dry_run"] = true.into();
    result["committed"] = false.into();
    result["note"]="Validated in an isolated temporary copy; no source mutation. Apply may conflict if source changes.".into();
    result.as_object_mut().unwrap().remove("commit");
    Ok(result)
}
fn dispatch(cli: &Cli) -> Result<Value> {
    let r = root(cli)?;
    match &cli.command {
        Commands::Capabilities => Ok(capabilities()),
        Commands::List {
            limit,
            max_bytes,
            cursor,
        } => {
            let v = library::list(&r)?;
            if limit.is_none() && max_bytes.is_none() && cursor.is_none() {
                Ok(v)
            } else {
                let rev = storage::hash(&serde_json::to_vec(&v)?);
                output::bounded(
                    json!({"items":v}),
                    "items",
                    limit.unwrap_or(20),
                    max_bytes.unwrap_or(8192),
                    cursor.as_deref(),
                    &format!("list:{}", r.canonicalize()?.display()),
                    &rev,
                )
            }
        }
        Commands::Init { base, title, owner } => library::initialize(&r, base, title, owner),
        Commands::Show {
            base,
            page,
            section,
            with_revision,
        } => pages::show(
            &Base::open(&r, base)?,
            page.as_deref(),
            section.as_deref(),
            *with_revision,
        ),
        Commands::Read {
            base,
            page,
            section,
            max_bytes,
        } => output::single(
            pages::show(
                &Base::open(&r, base)?,
                page.as_deref(),
                section.as_deref(),
                true,
            )?,
            *max_bytes,
        ),
        Commands::Outline { base, page, bounds } => {
            let b = Base::open(&r, base)?;
            let v = pages::outline(&b, page)?;
            paged(&b, v, "sections", bounds, &format!("outline:{page}"))
        }
        Commands::Search {
            base,
            query,
            mode,
            verified,
            concept,
            limit,
            max_bytes,
        } => {
            let b = Base::open(&r, base)?;
            if *mode == Mode::Literal
                && !verified
                && concept.is_none()
                && limit.is_none()
                && max_bytes.is_none()
            {
                search::literal(&b, query)
            } else {
                search::search(
                    &b,
                    query,
                    &SearchOptions {
                        ranked: *mode == Mode::Ranked,
                        verified: *verified,
                        concept: concept.clone(),
                        limit: limit.unwrap_or(8),
                        max_bytes: max_bytes.unwrap_or(8192),
                        cache_root: None,
                    },
                )
            }
        }
        Commands::Index { base, rebuild } => search::index(&Base::open(&r, base)?, *rebuild, None),
        Commands::Context {
            base,
            query,
            concept,
            mode,
            verified,
            limit,
            max_bytes,
        } => search::context(
            &Base::open(&r, base)?,
            query,
            &SearchOptions {
                ranked: *mode == Mode::Ranked,
                verified: *verified,
                limit: *limit,
                max_bytes: *max_bytes,
                concept: concept.clone(),
                cache_root: None,
            },
        ),
        Commands::Validate { base } => {
            let ids = if let Some(id) = base {
                vec![id.clone()]
            } else {
                let (_, c) = library::catalog(&r)?;
                c["knowledge_bases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v["id"].as_str().unwrap().to_string())
                    .collect()
            };
            let mut ok = Vec::new();
            let mut errors = Vec::new();
            for id in ids {
                let result = (|| {
                    let b = Base::open(&r, &id)?;
                    let summary = pages::validate(&b)?;
                    if b.root.join("data/memory/profile.json").exists() {
                        memory::validate(&b)?;
                    }
                    Ok::<Value, anyhow::Error>(summary)
                })();
                match result {
                    Ok(v) => ok.push(v),
                    Err(e) => errors.push(json!({"id":id,"error":format!("{e:#}")})),
                }
            }
            Ok(json!({"valid":errors.is_empty(),"knowledge_bases":ok,"errors":errors}))
        }
        Commands::Export {
            base,
            output,
            format,
        } => {
            let b = Base::open(&r, base)?;
            if b.root.join("data/memory/profile.json").exists() {
                memory::validate(&b)?;
            }
            pages::export(&b, output, format)
        }
        Commands::Page(cmd) => match cmd {
            PageCommand::Create {
                base,
                id,
                title,
                input,
            } => pages::create(&mut Base::open(&r, base)?, id, title, &text_input(input)?),
            PageCommand::Replace {
                base,
                id,
                expect_revision,
                input,
            } => pages::replace(
                &mut Base::open(&r, base)?,
                id,
                &text_input(input)?,
                expect_revision,
            ),
            PageCommand::Patch {
                base,
                id,
                expect_revision,
                input: p,
            } => pages::patch(&mut Base::open(&r, base)?, id, &input(p)?, expect_revision),
        },
        Commands::Edit { base, page } => {
            let b = Base::open(&r, base)?;
            let body = pages::contents(&b, page)?;
            let rev = storage::hash(body.as_bytes());
            drop(b);
            let mut draft = tempfile::Builder::new()
                .prefix("tars-kb-edit-")
                .suffix(".md")
                .tempfile()?;
            draft.write_all(body.as_bytes())?;
            let editor=std::env::var("VISUAL").or_else(|_|std::env::var("EDITOR")).context("editor_required: set EDITOR to an executable and optional whitespace-separated arguments")?;
            let args: Vec<_> = editor.split_whitespace().collect();
            ensure!(!args.is_empty(), "editor_required");
            let result = Command::new(args[0])
                .args(&args[1..])
                .arg(draft.path())
                .status()?;
            ensure!(result.success(), "editor_failed: edit was not committed");
            let body = String::from_utf8(fs::read(draft.path())?)?;
            match pages::replace(&mut Base::open(&r, base)?, page, &body, &rev) {
                Ok(v) => Ok(v),
                Err(e) => {
                    let (_, path) = draft.keep()?;
                    bail!("{e}; draft preserved at {}", path.display())
                }
            }
        }
        Commands::Source(SourceCommand::Add { base, input: p }) => {
            library::source_add(&mut Base::open(&r, base)?, input(p)?)
        }
        Commands::Memory(MemoryCommand::Init { base, profile }) => {
            ensure!(
                profile == memory::PROFILE,
                "unsupported_profile: supported profile is {}",
                memory::PROFILE
            );
            memory::memory_init(&mut Base::open(&r, base)?)
        }
        Commands::Concepts { base, bounds } => {
            let b = Base::open(&r, base)?;
            let v = memory::concept_list(&b)?;
            paged(&b, v, "items", bounds, "concepts")
        }
        Commands::Concept(cmd) => match cmd {
            ConceptCommand::Init {
                base,
                id,
                input: p,
                title,
                summary,
            } => {
                let value = if let Some(p) = p {
                    ensure!(
                        title.is_none() && summary.is_none(),
                        "invalid_input: choose --input or --title/--summary"
                    );
                    input(p)?
                } else {
                    json!({"title":title.as_deref().context("invalid_input: --title required")?,"summary":summary.as_deref().context("invalid_input: --summary required")?})
                };
                memory::concept_init(&mut Base::open(&r, base)?, id, value)
            }
            ConceptCommand::Get {
                base,
                id,
                max_bytes,
            } => output::single(memory::concept_get(&Base::open(&r, base)?, id)?, *max_bytes),
        },
        Commands::Remember {
            base,
            concept,
            input: p,
            idempotency_key,
            dry_run: dry,
        } => {
            let mut b = Base::open(&r, base)?;
            let mut v = input(p)?;
            ensure!(
                v.is_object(),
                "invalid_input: assertion input must be an object"
            );
            if let Some(c) = concept {
                if v.get("concepts").is_some() {
                    ensure!(
                        v["concepts"]
                            .as_array()
                            .is_some_and(|a| a.iter().any(|x| x == c)),
                        "invalid_input: positional concept disagrees with input"
                    );
                } else {
                    v["concepts"] = json!([c]);
                }
            }
            if *dry {
                dry_run(
                    &b,
                    vec![json!({"op":"remember","input":v})],
                    idempotency_key.as_deref(),
                )
            } else {
                memory::remember(&mut b, v, idempotency_key.as_deref())
            }
        }
        Commands::Revise {
            base,
            id,
            mode,
            expect_revision,
            input: p,
            idempotency_key,
            dry_run: dry,
        } => {
            let mut b = Base::open(&r, base)?;
            let v = input(p)?;
            if *dry {
                dry_run(
                    &b,
                    vec![
                        json!({"op":"revise","id":id,"mode":mode,"expect_revision":expect_revision,"input":v}),
                    ],
                    idempotency_key.as_deref(),
                )
            } else {
                memory::revise(
                    &mut b,
                    id,
                    mode,
                    v,
                    Some(expect_revision),
                    idempotency_key.as_deref(),
                )
            }
        }
        Commands::Fact {
            base,
            id,
            evidence,
            max_bytes,
        } => {
            let mut v = memory::fact(&Base::open(&r, base)?, id)?;
            if !evidence {
                v.as_object_mut().unwrap().remove("source_snapshots");
            }
            output::single(v, *max_bytes)
        }
        Commands::Facts {
            base,
            concept,
            valid_at,
            known_at,
            known_revision,
            view,
            bounds,
        } => {
            let b = Base::open(&r, base)?;
            let filter = FactFilter {
                valid_at: valid_at.clone(),
                known_at: known_at.clone(),
                known_revision: *known_revision,
                view: view.clone(),
            };
            let v = memory::facts(&b, concept.as_deref(), &filter)?;
            paged(
                &b,
                v,
                "items",
                bounds,
                &format!("facts:{concept:?}:{}", serde_json::to_string(&filter)?),
            )
        }
        Commands::History { base, id, bounds } => {
            let b = Base::open(&r, base)?;
            let v = memory::history(&b, id)?;
            paged(&b, v, "revisions", bounds, &format!("history:{id}"))
        }
        Commands::Batch {
            base,
            input: p,
            idempotency_key,
            dry_run: dry,
        } => {
            let v = input(p)?;
            let ops = v
                .as_array()
                .or_else(|| v["operations"].as_array())
                .context("invalid_input: batch requires an array or operations array")?
                .clone();
            let mut b = Base::open(&r, base)?;
            if *dry {
                dry_run(&b, ops, idempotency_key.as_deref())
            } else {
                memory::batch(&mut b, ops, idempotency_key.as_deref())
            }
        }
        Commands::Recover { base } => {
            let b = Base::open(&r, base)?;
            Ok(json!({"base_id":base,"status":"clean","revision":b.revision()?}))
        }
        Commands::Spec(cmd) => spec_command(cmd),
        Commands::Schema(cmd) => schema_command(cmd),
        Commands::Time(cmd) => time_command(cmd),
    }
}
fn spec_command(cmd: &SpecCommand) -> Result<Value> {
    let names = ["contract", "storage", "memory", "output"];
    match cmd {
        SpecCommand::List => Ok(json!({"specifications":names})),
        SpecCommand::Show { name, section } => {
            let body = match name.as_str() {
                "contract" => include_str!("../specs/contract.md"),
                "storage" => include_str!("../specs/storage.md"),
                "memory" => include_str!("../specs/memory.md"),
                "output" => include_str!("../specs/output.md"),
                _ => bail!("unknown_spec: use spec list"),
            };
            let content = if let Some(section) = section {
                let hs = pages::outline_text(body);
                let n = hs
                    .iter()
                    .position(|h| h["id"] == *section)
                    .context("not_found: spec section")?;
                let level = hs[n]["level"].as_u64().unwrap();
                let start = hs[n]["byte_start"].as_u64().unwrap() as usize;
                let end = hs[n + 1..]
                    .iter()
                    .find(|h| h["level"].as_u64().unwrap() <= level)
                    .map_or(body.len(), |h| h["byte_start"].as_u64().unwrap() as usize);
                &body[start..end]
            } else {
                body
            };
            Ok(json!({"name":name,"content":content}))
        }
    }
}
fn schema_command(cmd: &SchemaCommand) -> Result<Value> {
    match cmd {
        SchemaCommand::List => Ok(
            json!({"schemas":["catalog","knowledge-base","memory","profile","assertion","concept","content","temporal"]}),
        ),
        SchemaCommand::Show { name } => match name.as_str() {
            "catalog" => {
                storage::parse_json(include_bytes!("../specs/schemas/catalog.schema.json"))
            }
            "knowledge-base" => storage::parse_json(include_bytes!(
                "../specs/schemas/knowledge-base.schema.json"
            )),
            "memory" => storage::parse_json(include_bytes!("../schemas/memory.schema.json")),
            "profile" | "assertion" | "concept" | "content" | "temporal" => {
                let mut s = storage::parse_json(include_bytes!("../schemas/memory.schema.json"))?;
                s.as_object_mut().unwrap().remove("oneOf");
                s["$ref"] = format!("#/$defs/{name}").into();
                Ok(s)
            }
            _ => bail!("unknown_schema: use schema list"),
        },
    }
}

fn time_command(cmd: &TimeCommand) -> Result<Value> {
    match cmd {
        TimeCommand::Normalize { value } => {
            Ok(json!({"type":"instant","value":temporal::timestamp(value)?.to_string()}))
        }
        TimeCommand::Resolve {
            value,
            zone,
            reference,
        } => {
            use jiff::ToSpan;
            if ["today", "yesterday", "tomorrow"].contains(&value.as_str()) {
                let zone = zone
                    .as_deref()
                    .context("timezone_required: relative dates require --zone")?;
                let reference = reference
                    .as_deref()
                    .context("reference_required: relative dates require --reference")?;
                let at = temporal::timestamp(reference)?;
                let date = at.in_tz(zone)?.date();
                let date = match value.as_str() {
                    "yesterday" => date.checked_sub(1.day())?,
                    "tomorrow" => date.checked_add(1.day())?,
                    _ => date,
                };
                return Ok(
                    json!({"type":"date","value":date.to_string(),"time_zone":zone,"original":value,"reference":at.to_string()}),
                );
            }
            if value.len() == 10 {
                value.parse::<jiff::civil::Date>()?;
                if let Some(zone) = zone {
                    jiff::tz::TimeZone::get(zone)?;
                }
                let mut point = json!({"type":"date","value":value});
                if let Some(zone) = zone {
                    point["time_zone"] = zone.clone().into();
                }
                return Ok(point);
            }
            if let Ok(at) = temporal::timestamp(value) {
                return Ok(json!({"type":"instant","value":at.to_string(),"original":value}));
            }
            let zone = zone
                .as_deref()
                .context("timezone_required: local datetime requires --zone")?;
            ensure!(
                value.len() >= 19
                    && matches!(value.as_bytes().get(10), Some(b'T' | b't'))
                    && value.as_bytes()[11..]
                        .iter()
                        .all(|b| b.is_ascii_digit() || matches!(b, b':' | b'.')),
                "invalid_time: local datetime must be offset-free YYYY-MM-DDTHH:MM:SS; offset-qualified input must be RFC 3339"
            );
            ensure!(
                !value.contains(":60"),
                "invalid_time: leap seconds are not supported"
            );
            let civil = value.parse::<jiff::civil::DateTime>()?;
            let z=jiff::tz::TimeZone::get(zone)?.to_ambiguous_zoned(civil).unambiguous().context("ambiguous_local_time: supply an offset-qualified instant for DST fold; nonexistent local times are rejected")?;
            Ok(
                json!({"type":"instant","value":z.timestamp().to_string(),"source_time_zone":zone,"original":value}),
            )
        }
    }
}
fn main() {
    let cli = Cli::parse();
    let result = dispatch(&cli);
    match result {
        Ok(value) => {
            let invalid =
                matches!(cli.command, Commands::Validate { .. }) && value["valid"] == false;
            let rendered = if cli.json || value.get("truncated").is_some() {
                serde_json::to_string(&value).unwrap()
            } else {
                match &cli.command {
                    Commands::Show { .. } => value["content"].as_str().unwrap_or("").into(),
                    Commands::Spec(SpecCommand::Show { .. }) => {
                        value["content"].as_str().unwrap_or("").into()
                    }
                    Commands::List {
                        limit: None,
                        max_bytes: None,
                        cursor: None,
                    } => value
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|x| {
                            format!(
                                "{}\t{}\t{}\t{}",
                                x["id"].as_str().unwrap(),
                                x["title"].as_str().unwrap(),
                                x["version"].as_str().unwrap(),
                                x["visibility"].as_str().unwrap()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => serde_json::to_string_pretty(&value).unwrap(),
                }
            };
            let mut out = io::stdout().lock();
            if let Err(e) = writeln!(out, "{rendered}")
                && e.kind() != io::ErrorKind::BrokenPipe
            {
                eprintln!("tars-kb: {e}");
                std::process::exit(1);
            }
            if invalid {
                std::process::exit(1);
            }
        }
        Err(e) => {
            let message = format!("{e:#}");
            let candidate = message.split(':').next().unwrap_or("error");
            let code = if candidate
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'_')
            {
                candidate
            } else {
                "error"
            };
            if cli.json {
                println!("{}", json!({"error":message,"code":code}));
            } else {
                eprintln!("tars-kb: {message}");
            }
            std::process::exit(1);
        }
    }
}

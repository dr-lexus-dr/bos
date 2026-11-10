//! bpm — bOS package manager.
//!
//! Installs applications into the bOS file structure (`applications/`)
//! from the official bOS repository — https://github.com/bOS-operating-system/bos-repository
//! — or directly from any GitHub repository.
//!
//! The official repository exposes a JSON index at its root, `registry.json`:
//!
//! ```json
//! {
//!   "packages": [
//!     {
//!       "name": "calculator",
//!       "version": "1.0.0",
//!       "description": "Simple calculator",
//!       "category": "utilities",
//!       "repo": "bOS-operating-system/bos-apps",
//!       "ref": "main",
//!       "path": "packages/calculator",
//!       "sha256": "optional-tarball-checksum",
//!       "bin": ["calculator.sh"],
//!       "license": "MIT",
//!       "homepage": "https://…",
//!       "requires": ["another-package"]
//!     }
//!   ]
//! }
//! ```
//!
//! Packages live either in the official repository itself (`path` points
//! into the tarball of `repo@ref`) or in any other GitHub repository.
//!
//! bOS file structure used by bpm:
//!   applications/        installed packages
//!   system/bpm/          registry cache + install database
//!   system/logs/bpm.log         operation log
//!
//! Commands:
//!   bpm help
//!   bpm update                 refresh the registry from GitHub
//!   bpm list [category]        list available packages
//!   bpm search <query>         search name/description/category
//!   bpm info <name>            package details
//!   bpm install <spec>         install (registry name, owner/repo, or URL)
//!   bpm remove <name>          uninstall
//!   bpm installed              list installed packages
//!   bpm upgrade [name...]      update installed packages to registry versions

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BPM_VERSION: &str = "0.1.0";
const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/bOS-operating-system/bos-repository/main/registry.json";
/// Registry cache lifetime before bpm tries to refresh from GitHub.
const REGISTRY_FRESH_SECS: u64 = 3600;
const CACHE_DIR: &str = "system/bpm/cache";
const DB_PATH: &str = "system/bpm/installed.json";
const LOG_DIR: &str = "system/logs";
const APPS_DIR: &str = "applications";

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PackageEntry {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    category: String,
    /// GitHub "owner/repo".
    repo: String,
    /// Branch, tag or commit. Defaults to "main".
    #[serde(default = "default_ref", rename = "ref")]
    git_ref: String,
    /// Directory inside the tarball. Defaults to "." (whole repo).
    #[serde(default = "default_path")]
    path: String,
    /// Optional sha256 of the downloaded tarball.
    #[serde(default)]
    sha256: String,
    /// Files to mark executable after install.
    #[serde(default)]
    bin: Vec<String>,
    #[serde(default)]
    license: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    requires: Vec<String>,
}

fn default_ref() -> String {
    "main".into()
}

fn default_path() -> String {
    ".".into()
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct Registry {
    #[serde(default)]
    packages: Vec<PackageEntry>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct InstalledRecord {
    name: String,
    version: String,
    source: String,
    installed_at: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct InstalledDb {
    packages: Vec<InstalledRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RegistryCache {
    fetched_at: u64,
    registry: Registry,
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

fn usage() {
    println!("bpm v{BPM_VERSION} — bOS package manager");
    println!();
    println!("Usage: bpm <command> [args]");
    println!();
    println!("Commands:");
    println!("  update                  refresh the registry from GitHub");
    println!("  list [category]         list available packages (-f to force refresh)");
    println!("  search <query>          search name/description/category");
    println!("  info <name>             show package details");
    println!("  install <spec> [-f]     install a package");
    println!("      spec: registry name | owner/repo[@ref][:path] | https://github.com/owner/repo");
    println!("      -f, --force         reinstall even if already installed");
    println!("      --ref=<ref>         override the git ref (branch/tag/commit)");
    println!("      --path=<dir>        override the package directory inside the repo");
    println!("  remove <name>           uninstall a package");
    println!("  installed               list installed packages");
    println!("  upgrade [name...]       update installed packages to registry versions");
    println!("  version                 show bpm version");
    println!();
    println!("Environment:");
    println!("  BOS_ROOT                bOS root (default: auto-detected)");
    println!("  BPM_REGISTRY            registry.json URL override");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        return;
    }
    let cmd = args[0].as_str();
    let rest = &args[1..];
    let result = match cmd {
        "help" | "-h" | "--help" => {
            usage();
            Ok(())
        }
        "version" | "-V" => {
            println!("bpm v{BPM_VERSION}");
            Ok(())
        }
        "update" => cmd_update(rest),
        "list" | "ls" => cmd_list(rest),
        "search" | "find" => cmd_search(rest),
        "info" | "show" => cmd_info(rest),
        "install" | "add" => cmd_install(rest),
        "remove" | "rm" | "uninstall" => cmd_remove(rest),
        "installed" => cmd_installed(rest),
        "upgrade" => cmd_upgrade(rest),
        _ => {
            eprintln!("bpm: unknown command '{cmd}' — see 'bpm help'");
            exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("bpm: {e:#}");
        exit(1);
    }
}

// ---------------------------------------------------------------------------
// bOS root, HTTP client, time
// ---------------------------------------------------------------------------

/// Locate the bOS root: $BOS_ROOT, else the nearest ancestor containing the
/// three root directories (applications, system, users), else $CWD.
fn bos_root() -> PathBuf {
    if let Ok(r) = std::env::var("BOS_ROOT") {
        return PathBuf::from(r);
    }
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if dir.join("applications").is_dir()
            && dir.join("system").is_dir()
            && dir.join("users").is_dir()
        {
            return dir;
        }
        if !dir.pop() {
            return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        }
    }
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(format!("bpm/{BPM_VERSION} (bOS package manager)"))
        .timeout(Duration::from_secs(120))
        .build()
        .context("failed to build HTTP client")
}

fn registry_url() -> String {
    std::env::var("BPM_REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY_URL.into())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Howard Hinnant's days→(y,m,d) algorithm, for log timestamps without chrono.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn iso_now() -> String {
    let secs = now_secs();
    let (y, m, d) = civil_from_days((secs / 86400) as i64);
    let s = secs % 86400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, (s % 3600) / 60, s % 60)
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

fn fetch_registry(client: &reqwest::blocking::Client) -> Result<Registry> {
    let url = registry_url();
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("GET {url}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        // The official repository exists but has no registry.json yet.
        println!("bpm: official registry is empty (no registry.json yet)");
        println!("     add packages to {DEFAULT_REGISTRY_URL} or install directly:");
        println!("     bpm install owner/repo");
        return Ok(Registry::default());
    }
    let status = resp.status();
    let body = resp.text().context("reading registry body")?;
    if !status.is_success() {
        bail!("registry HTTP {status} at {url}");
    }
    serde_json::from_str(&body).with_context(|| format!("parsing registry at {url}"))
}

/// Load the registry: from cache when fresh, from GitHub otherwise,
/// falling back to the cache when offline.
fn load_registry(client: &reqwest::blocking::Client, root: &Path, force: bool) -> Result<Registry> {
    let cache = root.join(CACHE_DIR).join("registry.json");
    if !force {
        if let Ok(data) = fs::read_to_string(&cache) {
            if let Ok(c) = serde_json::from_str::<RegistryCache>(&data) {
                if now_secs().saturating_sub(c.fetched_at) < REGISTRY_FRESH_SECS {
                    return Ok(c.registry);
                }
            }
        }
    }
    match fetch_registry(client) {
        Ok(reg) => {
            if let Some(parent) = cache.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let c = RegistryCache {
                fetched_at: now_secs(),
                registry: reg.clone(),
            };
            if let Ok(s) = serde_json::to_string_pretty(&c) {
                let _ = fs::write(&cache, s);
            }
            Ok(reg)
        }
        Err(e) => {
            if let Ok(data) = fs::read_to_string(&cache) {
                if let Ok(c) = serde_json::from_str::<RegistryCache>(&data) {
                    eprintln!("bpm: registry fetch failed ({e:#}), using cached copy");
                    return Ok(c.registry);
                }
            }
            Err(e).context("cannot reach the registry (offline? try again later)")
        }
    }
}

// ---------------------------------------------------------------------------
// Install database & log
// ---------------------------------------------------------------------------

fn load_db(root: &Path) -> InstalledDb {
    fs::read_to_string(root.join(DB_PATH))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_db(root: &Path, db: &InstalledDb) -> Result<()> {
    let path = root.join(DB_PATH);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("creating system/bpm")?;
    }
    let s = serde_json::to_string_pretty(db).context("serializing install database")?;
    fs::write(&path, s).context("writing install database")
}

fn log(root: &Path, msg: &str) {
    let dir = root.join(LOG_DIR);
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("bpm.log");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {msg}", iso_now());
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_update(_args: &[String]) -> Result<()> {
    let root = bos_root();
    let client = client()?;
    let registry = load_registry(&client, &root, true)?;
    println!(
        "bpm: registry refreshed — {} package(s) available",
        registry.packages.len()
    );
    Ok(())
}

fn cmd_list(args: &[String]) -> Result<()> {
    let force = args.iter().any(|a| a == "-f" || a == "--force");
    let category = args.iter().find(|a| !a.starts_with('-')).cloned();
    let root = bos_root();
    let client = client()?;
    let registry = load_registry(&client, &root, force)?;

    let mut pkgs: Vec<&PackageEntry> = registry
        .packages
        .iter()
        .filter(|p| {
            category
                .as_deref()
                .map(|c| p.category.eq_ignore_ascii_case(c))
                .unwrap_or(true)
        })
        .collect();
    pkgs.sort_by(|a, b| a.name.cmp(&b.name));

    if pkgs.is_empty() {
        println!(
            "bpm: no packages{}",
            category.as_deref().map(|c| format!(" in category '{c}'")).unwrap_or_default()
        );
        println!("tip: install directly from GitHub with: bpm install owner/repo");
        return Ok(());
    }

    let db = load_db(&root);
    println!("{:<28} {:<10} {:<12} {}", "NAME", "VERSION", "CATEGORY", "DESCRIPTION");
    println!("{}", "-".repeat(100));
    for p in pkgs {
        let installed = db.packages.iter().any(|r| r.name == p.name);
        let marker = if installed { "*" } else { " " };
        println!(
            "{:<28}{} {:<10} {:<12} {}",
            p.name, marker, p.version, p.category, p.description
        );
    }
    let installed_count = db.packages.len();
    println!();
    println!(
        "* = installed ({} installed package{})",
        installed_count,
        if installed_count == 1 { "" } else { "s" }
    );
    Ok(())
}

fn cmd_search(args: &[String]) -> Result<()> {
    let query = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .cloned()
        .ok_or_else(|| anyhow!("usage: bpm search <query>"))?
        .to_lowercase();
    let root = bos_root();
    let client = client()?;
    let registry = load_registry(&client, &root, false)?;

    let mut hits: Vec<&PackageEntry> = registry
        .packages
        .iter()
        .filter(|p| {
            p.name.to_lowercase().contains(&query)
                || p.description.to_lowercase().contains(&query)
                || p.category.to_lowercase().contains(&query)
        })
        .collect();
    hits.sort_by(|a, b| a.name.cmp(&b.name));

    if hits.is_empty() {
        println!("bpm: no packages match '{query}'");
        return Ok(());
    }
    println!("{:<28} {:<10} {}", "NAME", "VERSION", "DESCRIPTION");
    println!("{}", "-".repeat(90));
    for p in &hits {
        println!("{:<28} {:<10} {}", p.name, p.version, p.description);
    }
    println!("\n{} match(es)", hits.len());
    Ok(())
}

fn cmd_info(args: &[String]) -> Result<()> {
    let name = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .cloned()
        .ok_or_else(|| anyhow!("usage: bpm info <name>"))?;
    let root = bos_root();
    let client = client()?;
    let registry = load_registry(&client, &root, false)?;
    let entry = registry
        .packages
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| anyhow!("package '{name}' not found in the registry"))?;

    println!("Name:        {}", entry.name);
    println!("Version:     {}", entry.version);
    println!("Description: {}", entry.description);
    println!("Category:    {}", entry.category);
    println!("Repository:  github.com/{}", entry.repo);
    println!("Ref:         {}", entry.git_ref);
    if entry.path != "." {
        println!("Path:        {}", entry.path);
    }
    if !entry.license.is_empty() {
        println!("License:     {}", entry.license);
    }
    if !entry.homepage.is_empty() {
        println!("Homepage:    {}", entry.homepage);
    }
    if !entry.bin.is_empty() {
        println!("Binaries:    {}", entry.bin.join(", "));
    }
    if !entry.requires.is_empty() {
        println!("Depends on:  {}", entry.requires.join(", "));
    }
    if !entry.sha256.is_empty() {
        println!("SHA256:      {}", entry.sha256);
    }

    let db = load_db(&root);
    match db.packages.iter().find(|r| r.name == entry.name) {
        Some(r) => println!(
            "\nInstalled:   v{} from {} ({})",
            r.version, r.source, r.installed_at
        ),
        None => println!("\nInstalled:   no"),
    }
    Ok(())
}

fn cmd_installed(_args: &[String]) -> Result<()> {
    let root = bos_root();
    let db = load_db(&root);
    if db.packages.is_empty() {
        println!("bpm: no packages installed");
        return Ok(());
    }
    println!("{:<28} {:<10} {}", "NAME", "VERSION", "SOURCE");
    println!("{}", "-".repeat(80));
    for r in &db.packages {
        println!("{:<28} {:<10} {}", r.name, r.version, r.source);
    }
    println!("\n{} installed package(s)", db.packages.len());
    Ok(())
}

fn cmd_remove(args: &[String]) -> Result<()> {
    let name = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .cloned()
        .ok_or_else(|| anyhow!("usage: bpm remove <name>"))?;
    let root = bos_root();
    let mut db = load_db(&root);
    let before = db.packages.len();
    db.packages.retain(|r| r.name != name);
    if db.packages.len() == before {
        bail!("package '{name}' is not installed");
    }
    save_db(&root, &db)?;
    let dst = root.join(APPS_DIR).join(&name);
    if dst.exists() {
        fs::remove_dir_all(&dst).with_context(|| format!("removing {}", dst.display()))?;
    }
    log(&root, &format!("remove {name}"));
    println!("bpm: removed '{name}'");
    Ok(())
}

fn cmd_upgrade(args: &[String]) -> Result<()> {
    let only: Vec<String> = args.iter().filter(|a| !a.starts_with('-')).cloned().collect();
    let root = bos_root();
    let client = client()?;
    let registry = load_registry(&client, &root, false)?;
    let mut db = load_db(&root);

    let targets: Vec<String> = if only.is_empty() {
        db.packages.iter().map(|r| r.name.clone()).collect()
    } else {
        only
    };

    let mut upgraded = 0;
    for name in &targets {
        let rec = match db.packages.iter().find(|r| r.name == *name) {
            Some(r) => r.clone(),
            None => {
                eprintln!("bpm: '{name}' is not installed");
                continue;
            }
        };
        let entry = match registry.packages.iter().find(|p| p.name == *name) {
            Some(p) => p,
            None => {
                eprintln!("bpm: '{name}' is no longer in the registry");
                continue;
            }
        };
        if entry.version == rec.version {
            println!("bpm: '{name}' is up to date (v{})", entry.version);
            continue;
        }
        println!(
            "bpm: upgrading '{name}' v{} → v{}",
            rec.version, entry.version
        );
        let mut done = HashSet::new();
        install_one(&client, &root, &registry, name, true, None, None, &mut done)?;
        upgraded += 1;
    }
    println!("bpm: {upgraded} package(s) upgraded");
    Ok(())
}

fn cmd_install(args: &[String]) -> Result<()> {
    let mut specs = Vec::new();
    let mut force = false;
    let mut git_ref: Option<String> = None;
    let mut path_override: Option<String> = None;
    for a in args {
        match a.as_str() {
            "-f" | "--force" => force = true,
            s if s.starts_with("--ref=") => git_ref = Some(s["--ref=".len()..].into()),
            s if s.starts_with("--path=") => path_override = Some(s["--path=".len()..].into()),
            s if s.starts_with('-') => bail!("unknown option '{s}' — see 'bpm help'"),
            s => specs.push(s.to_string()),
        }
    }
    if specs.is_empty() {
        bail!("usage: bpm install <package|owner/repo|url>");
    }
    let root = bos_root();
    let client = client()?;
    let registry = load_registry(&client, &root, false)?;

    let mut done = HashSet::new();
    for spec in &specs {
        install_one(&client, &root, &registry, spec, force, git_ref.as_deref(), path_override.as_deref(), &mut done)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

/// Resolve a spec to a package entry: registry name first, then a direct
/// GitHub reference (owner/repo[@ref][:path] or a github.com URL).
fn resolve_spec(
    registry: &Registry,
    spec: &str,
    git_ref: Option<&str>,
    path_override: Option<&str>,
) -> Result<PackageEntry> {
    if let Some(p) = registry.packages.iter().find(|p| p.name == spec) {
        let mut e = p.clone();
        if let Some(r) = git_ref {
            e.git_ref = r.into();
        }
        if let Some(pth) = path_override {
            e.path = pth.into();
        }
        return Ok(e);
    }
    parse_direct(spec, git_ref, path_override)
}

fn parse_direct(
    spec: &str,
    git_ref: Option<&str>,
    path_override: Option<&str>,
) -> Result<PackageEntry> {
    let mut repo_part = spec.trim().to_string();
    let mut ref_part: Option<String> = None;
    let mut path_part: Option<String> = None;

    if repo_part.starts_with("https://") || repo_part.starts_with("http://") {
        let after_scheme = repo_part.split("://").nth(1).unwrap_or("");
        let mut parts = after_scheme.splitn(2, '/');
        let host = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");
        if !host.ends_with("github.com") {
            bail!("bpm only supports github.com URLs, got '{host}'");
        }
        let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        let owner = segs.first().copied().unwrap_or("");
        let repo = segs.get(1).copied().unwrap_or("");
        if owner.is_empty() || repo.is_empty() {
            bail!("'{spec}' is not a valid GitHub repository URL");
        }
        // optional /tree/<ref>/<path...> or /blob/<ref>/<path...>
        let tail = &segs[2.min(segs.len())..];
        if tail.first().copied() == Some("tree") || tail.first().copied() == Some("blob") {
            if tail.len() >= 2 {
                ref_part = Some(tail[1].to_string());
                if tail.len() > 2 {
                    path_part = Some(tail[2..].join("/"));
                }
            }
        }
        repo_part = format!("{owner}/{repo}");
    } else {
        // owner/repo[@ref][:path]
        if let Some((a, b)) = repo_part.split_once(':') {
            path_part = Some(b.to_string());
            repo_part = a.to_string();
        }
        if let Some((a, b)) = repo_part.split_once('@') {
            ref_part = Some(b.to_string());
            repo_part = a.to_string();
        }
    }

    let repo = repo_part.trim_end_matches(".git").trim_end_matches('/').to_string();
    if !repo.contains('/') {
        bail!("'{spec}' is not in the registry and does not look like owner/repo");
    }
    let name = repo
        .rsplit('/')
        .next()
        .unwrap_or(&repo)
        .trim_end_matches(".git")
        .to_string();

    let git_ref_final = git_ref
        .or(ref_part.as_deref())
        .map(str::to_owned)
        .unwrap_or_else(default_ref);
    let path_final = path_override
        .or(path_part.as_deref())
        .map(str::to_owned)
        .unwrap_or_else(default_path);
    Ok(PackageEntry {
        name,
        version: git_ref_final.clone(),
        description: format!("Direct install from github.com/{repo}"),
        category: "direct".into(),
        repo: repo.clone(),
        git_ref: git_ref_final,
        path: path_final,
        sha256: String::new(),
        bin: Vec::new(),
        license: String::new(),
        homepage: format!("https://github.com/{repo}"),
        requires: Vec::new(),
    })
}

fn install_one(
    client: &reqwest::blocking::Client,
    root: &Path,
    registry: &Registry,
    spec: &str,
    force: bool,
    git_ref: Option<&str>,
    path_override: Option<&str>,
    done: &mut HashSet<String>,
) -> Result<()> {
    if !done.insert(spec.to_string()) {
        return Ok(()); // dependency cycle guard
    }
    let entry = resolve_spec(registry, spec, git_ref, path_override)?;

    // Install dependencies first.
    for dep in entry.requires.clone() {
        if registry.packages.iter().any(|p| p.name == dep) {
            install_one(client, root, registry, &dep, force, None, None, done)?;
        } else {
            eprintln!("bpm: warning: '{spec}' requires '{dep}', which is not in the registry");
        }
    }

    let dst = root.join(APPS_DIR).join(&entry.name);
    if dst.exists() {
        if !force {
            let db = load_db(root);
            match db.packages.iter().find(|r| r.name == entry.name) {
                Some(r) => {
                    println!(
                        "bpm: '{}' v{} already installed (registry has v{}) — use -f to reinstall",
                        entry.name, r.version, entry.version
                    );
                    return Ok(());
                }
                None => {
                    println!(
                        "bpm: '{}' already exists in applications/ — use -f to overwrite",
                        entry.name
                    );
                    return Ok(());
                }
            }
        }
        fs::remove_dir_all(&dst).with_context(|| format!("removing {}", dst.display()))?;
    }

    println!(
        "bpm: installing {} v{} from github.com/{} (ref {})…",
        entry.name, entry.version, entry.repo, entry.git_ref
    );

    let tarball = download_tarball(client, &entry)?;
    if !entry.sha256.trim().is_empty() {
        verify_sha256(&tarball, &entry.sha256)?;
    }

    let tmp = root.join(CACHE_DIR).join(format!("tmp-{}", entry.name));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).context("creating extraction directory")?;
    let src = extract_package(&tarball, &entry, &tmp)?;

    fs::create_dir_all(root.join(APPS_DIR)).context("creating applications/")?;
    copy_dir(&src, &dst)?;
    let _ = fs::remove_dir_all(&tmp);

    #[cfg(unix)]
    make_executable(&dst, &entry.bin)?;

    let mut db = load_db(root);
    db.packages.retain(|r| r.name != entry.name);
    db.packages.push(InstalledRecord {
        name: entry.name.clone(),
        version: entry.version.clone(),
        source: format!("github:{}/{}", entry.repo, entry.git_ref),
        installed_at: iso_now(),
    });
    save_db(root, &db)?;
    log(
        root,
        &format!("install {} v{} ({}#{})", entry.name, entry.version, entry.repo, entry.git_ref),
    );
    println!("bpm: installed '{}' v{} → applications/{}", entry.name, entry.version, entry.name);
    Ok(())
}

fn download_tarball(client: &reqwest::blocking::Client, entry: &PackageEntry) -> Result<Vec<u8>> {
    // codeload serves the repo archive; try branch, tag, then raw ref.
    let candidates = [
        format!(
            "https://codeload.github.com/{}/tar.gz/refs/heads/{}",
            entry.repo, entry.git_ref
        ),
        format!(
            "https://codeload.github.com/{}/tar.gz/refs/tags/{}",
            entry.repo, entry.git_ref
        ),
        format!("https://codeload.github.com/{}/tar.gz/{}", entry.repo, entry.git_ref),
    ];
    let mut last_err = String::new();
    for url in candidates {
        match client.get(&url).send() {
            Ok(r) if r.status().is_success() => {
                return r.bytes().map(|b| b.to_vec()).context("reading tarball");
            }
            Ok(r) => last_err = format!("HTTP {} for {url}", r.status()),
            Err(e) => last_err = e.to_string(),
        }
    }
    bail!(
        "download failed for {}@{} ({last_err})",
        entry.repo,
        entry.git_ref
    )
}

fn verify_sha256(bytes: &[u8], expected: &str) -> Result<()> {
    use sha2::{Digest, Sha256};
    let sum = Sha256::digest(bytes);
    let hex: String = sum.iter().map(|b| format!("{b:02x}")).collect();
    if hex != expected.trim() {
        bail!("sha256 mismatch: expected {}, got {}", expected.trim(), hex);
    }
    println!("bpm: sha256 verified");
    Ok(())
}

/// Unpack a GitHub tarball and return the package directory inside it.
fn extract_package(tarball: &[u8], entry: &PackageEntry, tmp: &Path) -> Result<PathBuf> {
    let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(tarball));
    let mut archive = tar::Archive::new(gz);
    archive.unpack(tmp).context("extracting tarball")?;
    let top = fs::read_dir(tmp)?
        .next()
        .ok_or_else(|| anyhow!("empty tarball"))??
        .path();
    let src = if entry.path == "." || entry.path.is_empty() {
        top
    } else {
        top.join(&entry.path)
    };
    if !src.is_dir() {
        bail!("path '{}' not found in the package", entry.path);
    }
    Ok(src)
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let target = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            fs::copy(&e.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(root: &Path, bins: &[String]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for b in bins {
        let p = root.join(b);
        if let Ok(meta) = fs::metadata(&p) {
            let mut perm = meta.permissions();
            perm.set_mode(0o755);
            fs::set_permissions(&p, perm)?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_root: &Path, _bins: &[String]) -> Result<()> {
    Ok(())
}

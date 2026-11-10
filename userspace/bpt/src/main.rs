//! bPT installs files and packages from GitHub into the bOS workspace.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::exit;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BPT_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/dr-lexus-dr/bos/main/~BPT-PROGS/registry.json";
/// Registry cache lifetime before bpt tries to refresh from GitHub.
const REGISTRY_FRESH_SECS: u64 = 3600;
const CACHE_DIR: &str = "system/bpt-data/cache";
const DB_PATH: &str = "system/bpt-data/installed.json";
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
    #[serde(default)]
    automatic: bool,
    #[serde(default)]
    catalog: bool,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    entry: Option<PackageEntry>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct InstalledDb {
    packages: Vec<InstalledRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RegistryCache {
    fetched_at: u64,
    #[serde(default)]
    url: String,
    registry: Registry,
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

fn usage() {
    println!("bpt v{BPT_VERSION} — bOS package manager");
    println!();
    println!("Usage: bpt <command> [args]");
    println!();
    println!("Commands:");
    println!("  update                  refresh the registry from GitHub");
    println!("  list [category]         list available packages (--installed, --upgradable)");
    println!("  search <query>          search name/description/category");
    println!("  info <name>             show package details");
    println!("  install <spec> [-f]     install a package");
    println!("      spec: registry name | owner/repo[@ref][:path] | https://github.com/owner/repo");
    println!("      -f, --force         reinstall even if already installed");
    println!("      --ref=<ref>         override the git ref (branch/tag/commit)");
    println!("      --path=<dir>        override the package directory inside the repo");
    println!("  reinstall <name>        reinstall a managed package");
    println!("  remove <name>...        uninstall packages");
    println!("  installed               list installed packages");
    println!("  upgrade [name...]       update installed packages to registry versions");
    println!("  autoremove              remove unused automatic dependencies");
    println!("  depends <name>           show catalog dependencies");
    println!("  download <spec>         download the source archive into the current directory");
    println!("  check                   check installed files and dependencies");
    println!("  clean                   clear the registry cache");
    println!("  version                 show bpt version");
    println!();
    println!("Environment:");
    println!("  BOS_ROOT                bOS root (default: auto-detected)");
    println!("  BPT_REGISTRY            registry.json URL override");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        return;
    }
    let cmd = args[0].as_str();
    let rest = &args[1..];
    let result = run(cmd, rest);
    if let Err(e) = result {
        eprintln!("bpt: {e:#}");
        exit(1);
    }
}

fn run(cmd: &str, rest: &[String]) -> Result<()> {
    let _lock = match cmd {
        "help" | "-h" | "--help" | "version" | "-V" | "--version" => None,
        _ => Some(lock_state(&bos_root())?),
    };
    match cmd {
        "help" | "-h" | "--help" => {
            usage();
            Ok(())
        }
        "version" | "-V" | "--version" => {
            println!("bpt v{BPT_VERSION}");
            Ok(())
        }
        "update" => cmd_update(rest),
        "list" | "ls" => cmd_list(rest),
        "search" | "find" => cmd_search(rest),
        "info" | "show" => cmd_info(rest),
        "install" | "add" => cmd_install(rest, false),
        "reinstall" => cmd_install(rest, true),
        "remove" | "rm" | "uninstall" => cmd_remove(rest),
        "installed" => cmd_installed(rest),
        "upgrade" => cmd_upgrade(rest),
        "autoremove" => cmd_autoremove(rest),
        "depends" => cmd_depends(rest),
        "check" => cmd_check(rest),
        "clean" => cmd_clean(rest),
        "download" => cmd_download(rest),
        _ => {
            eprintln!("bpt: unknown command '{cmd}' — see 'bpt help'");
            exit(2);
        }
    }
}

fn lock_state(root: &Path) -> Result<fs::File> {
    let state = root.join("system/bpt-data");
    fs::create_dir_all(&state)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("lock"))?;
    file.lock().context("locking bPT state")?;
    Ok(file)
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
        .user_agent(format!("bpt/{BPT_VERSION} (bOS package manager)"))
        .timeout(Duration::from_secs(120))
        .build()
        .context("failed to build HTTP client")
}

fn registry_url() -> String {
    std::env::var("BPT_REGISTRY")
        .or_else(|_| std::env::var("BPM_REGISTRY"))
        .unwrap_or_else(|_| DEFAULT_REGISTRY_URL.into())
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
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        (s % 3600) / 60,
        s % 60
    )
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
    let status = resp.status();
    let body = resp.text().context("reading registry body")?;
    if !status.is_success() {
        bail!("registry HTTP {status} at {url}");
    }
    let registry: Registry =
        serde_json::from_str(&body).with_context(|| format!("parsing registry at {url}"))?;
    let mut names = HashSet::new();
    for entry in &registry.packages {
        validate_entry(entry)?;
        if !names.insert(&entry.name) {
            bail!("duplicate package '{}' in registry", entry.name);
        }
    }
    Ok(registry)
}

/// Load the registry: from cache when fresh, from GitHub otherwise,
/// falling back to the cache when offline.
fn load_registry(client: &reqwest::blocking::Client, root: &Path, force: bool) -> Result<Registry> {
    let cache = root.join(CACHE_DIR).join("registry.json");
    let url = registry_url();
    if !force {
        if let Ok(data) = fs::read_to_string(&cache) {
            if let Ok(c) = serde_json::from_str::<RegistryCache>(&data) {
                if c.url == url && now_secs().saturating_sub(c.fetched_at) < REGISTRY_FRESH_SECS {
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
                url: url.clone(),
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
                    if force || c.url != url {
                        return Err(e);
                    }
                    eprintln!("bpt: registry fetch failed ({e:#}), using cached copy");
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

fn load_db(root: &Path) -> Result<InstalledDb> {
    let path = root.join(DB_PATH);
    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).context("invalid bPT installed database"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::read_to_string(root.join("system/bpm/installed.json")) {
                Ok(contents) => {
                    serde_json::from_str(&contents).context("invalid legacy package database")
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
                {
                    Ok(InstalledDb::default())
                }
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn save_db(root: &Path, db: &InstalledDb) -> Result<()> {
    let path = root.join(DB_PATH);
    let temp = path.with_extension("new");
    let file = fs::File::create(&temp)?;
    serde_json::to_writer_pretty(&file, db)?;
    file.sync_all()?;
    fs::rename(&temp, &path).context("writing install database")
}

fn log(root: &Path, msg: &str) {
    let dir = root.join(LOG_DIR);
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("bpt.log");
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
        "bpt: registry refreshed — {} package(s) available",
        registry.packages.len()
    );
    Ok(())
}

fn cmd_list(args: &[String]) -> Result<()> {
    if args == ["--installed"] {
        return cmd_installed(&[]);
    }
    if args == ["--upgradable"] {
        let root = bos_root();
        let db = load_db(&root)?;
        let registry = load_registry(&client()?, &root, false)?;
        for record in db
            .packages
            .iter()
            .filter(|record| record.catalog || record.entry.is_none())
        {
            if let Some(entry) = registry
                .packages
                .iter()
                .find(|entry| entry.name == record.name && entry.version != record.version)
            {
                println!("{} {} -> {}", record.name, record.version, entry.version);
            }
        }
        return Ok(());
    }
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
            "bpt: no packages{}",
            category
                .as_deref()
                .map(|c| format!(" in category '{c}'"))
                .unwrap_or_default()
        );
        println!("tip: install directly from GitHub with: bpt install owner/repo");
        return Ok(());
    }

    let db = load_db(&root)?;
    println!(
        "{:<28} {:<10} {:<12} {}",
        "NAME", "VERSION", "CATEGORY", "DESCRIPTION"
    );
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
        .ok_or_else(|| anyhow!("usage: bpt search <query>"))?
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
        println!("bpt: no packages match '{query}'");
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
        .ok_or_else(|| anyhow!("usage: bpt info <name>"))?;
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

    let db = load_db(&root)?;
    match db.packages.iter().find(|r| r.name == entry.name) {
        Some(r) => println!(
            "\nInstalled:   {} from {} ({})",
            r.version, r.source, r.installed_at
        ),
        None => println!("\nInstalled:   no"),
    }
    Ok(())
}

fn cmd_installed(_args: &[String]) -> Result<()> {
    let root = bos_root();
    let db = load_db(&root)?;
    if db.packages.is_empty() {
        println!("bpt: no packages installed");
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
    if args.is_empty() {
        bail!("usage: bpt remove <name>...");
    }
    let root = bos_root();
    let mut db = load_db(&root)?;
    let names: HashSet<&str> = args.iter().map(String::as_str).collect();
    for name in &names {
        validate_name(name)?;
        if !db.packages.iter().any(|record| record.name == *name) {
            bail!("package '{name}' is not installed");
        }
    }
    for record in &db.packages {
        if !names.contains(record.name.as_str()) {
            if let Some(dep) = record
                .requires
                .iter()
                .find(|dep| names.contains(dep.as_str()))
            {
                bail!("'{}' depends on '{dep}'", record.name);
            }
        }
    }
    remove_packages(&root, &mut db, names)
}

fn remove_packages(root: &Path, db: &mut InstalledDb, mut names: HashSet<&str>) -> Result<()> {
    while !names.is_empty() {
        // Remove dependants first so each successful operation leaves a usable DB.
        let name = names
            .iter()
            .copied()
            .find(|name| {
                !db.packages
                    .iter()
                    .any(|record| record.requires.iter().any(|dep| dep == name))
            })
            .ok_or_else(|| anyhow!("cannot remove packages with a dependency cycle"))?;
        let index = db
            .packages
            .iter()
            .position(|record| record.name == name)
            .ok_or_else(|| anyhow!("package '{name}' is not installed"))?;
        let dst = root.join(APPS_DIR).join(name);
        let work = work_dir(root, name)?;
        let backup = work.join("previous");
        let record = db.packages.remove(index);
        let result = replace_package(root, db, &dst, None, &backup);
        if result.is_err() {
            db.packages.insert(index, record);
        }
        finish_work(&work, &backup, result)?;
        names.remove(name);
        log(root, &format!("remove {name}"));
        println!("bpt: removed '{name}'");
    }
    Ok(())
}

fn cmd_upgrade(args: &[String]) -> Result<()> {
    let root = bos_root();
    let client = client()?;
    let registry = load_registry(&client, &root, false)?;
    let mut db = load_db(&root)?;
    let targets: Vec<_> = if args.is_empty() {
        db.packages
            .iter()
            .map(|record| record.name.clone())
            .collect()
    } else {
        args.to_vec()
    };
    let mut upgraded = 0;
    for name in targets {
        let record = db
            .packages
            .iter()
            .find(|record| record.name == name)
            .ok_or_else(|| anyhow!("package '{name}' is not installed"))?;
        if record.entry.is_some() && !record.catalog {
            continue;
        }
        let Some(entry) = registry.packages.iter().find(|entry| entry.name == name) else {
            continue;
        };
        if entry.version == record.version {
            continue;
        }
        let automatic = record.automatic;
        let mut installer = Installer {
            root: &root,
            client: &client,
            registry: &registry,
            db: &mut db,
            visiting: HashSet::new(),
        };
        installer.install(entry.clone(), true, automatic, true)?;
        upgraded += 1;
    }
    println!("bpt: {upgraded} package(s) upgraded");
    Ok(())
}

fn cmd_install(args: &[String], reinstall: bool) -> Result<()> {
    let mut specs = Vec::new();
    let mut force = reinstall;
    let mut git_ref = None;
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            "-f" | "--force" | "--reinstall" => force = true,
            arg if arg.starts_with("--ref=") => git_ref = arg.strip_prefix("--ref="),
            arg if arg.starts_with("--path=") => path = arg.strip_prefix("--path="),
            arg if arg.starts_with('-') => bail!("unknown option '{arg}'"),
            arg => specs.push(arg),
        }
    }
    if specs.is_empty() {
        bail!("usage: bpt install <package|owner/repo|url>...");
    }
    let root = bos_root();
    let client = client()?;
    let mut db = load_db(&root)?;
    let use_registry = specs.iter().any(|spec| {
        !spec.contains('/')
            && !db
                .packages
                .iter()
                .any(|record| record.name == *spec && !record.catalog && record.entry.is_some())
    });
    let registry = if use_registry {
        load_registry(&client, &root, false)?
    } else {
        Registry::default()
    };
    let mut installer = Installer {
        root: &root,
        client: &client,
        registry: &registry,
        db: &mut db,
        visiting: HashSet::new(),
    };
    for spec in specs {
        let cached = installer
            .db
            .packages
            .iter()
            .find(|record| record.name == spec && !record.catalog)
            .and_then(|record| record.entry.clone());
        let catalog = registry.packages.iter().find(|entry| entry.name == spec);
        let (entry, catalog) = match (cached, catalog) {
            (Some(entry), _) if git_ref.is_none() && path.is_none() => (entry, false),
            (_, Some(entry)) if git_ref.is_none() && path.is_none() => (entry.clone(), true),
            (_, Some(_)) => {
                bail!("use a GitHub source to override a catalog package's ref or path")
            }
            _ => (parse_direct(spec, git_ref, path)?, false),
        };
        if reinstall
            && !installer
                .db
                .packages
                .iter()
                .any(|record| record.name == entry.name)
        {
            bail!("package '{}' is not installed", entry.name);
        }
        installer.install(entry, catalog, false, force)?;
    }
    Ok(())
}

fn cmd_autoremove(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: bpt autoremove");
    }
    let root = bos_root();
    let mut db = load_db(&root)?;
    let mut queue: Vec<_> = db
        .packages
        .iter()
        .filter(|record| !record.automatic)
        .map(|record| record.name.as_str())
        .collect();
    let mut needed = HashSet::new();
    while let Some(name) = queue.pop() {
        if needed.insert(name) {
            if let Some(record) = db.packages.iter().find(|record| record.name == name) {
                queue.extend(record.requires.iter().map(String::as_str));
            }
        }
    }
    let unused: Vec<_> = db
        .packages
        .iter()
        .filter(|record| !needed.contains(record.name.as_str()))
        .map(|record| record.name.clone())
        .collect();
    if unused.is_empty() {
        println!("bpt: no unused dependencies");
    }
    remove_packages(&root, &mut db, unused.iter().map(String::as_str).collect())
}

fn cmd_check(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: bpt check");
    }
    let root = bos_root();
    let db = load_db(&root)?;
    for record in &db.packages {
        validate_name(&record.name)?;
        if !root.join(APPS_DIR).join(&record.name).is_dir() {
            bail!("missing files for '{}'", record.name);
        }
        for dep in &record.requires {
            if !db.packages.iter().any(|candidate| candidate.name == *dep) {
                bail!("'{}' depends on missing package '{dep}'", record.name);
            }
        }
    }
    println!("bpt: {} installed package(s) checked", db.packages.len());
    Ok(())
}

fn cmd_depends(args: &[String]) -> Result<()> {
    let [name] = args else {
        bail!("usage: bpt depends <name>");
    };
    let root = bos_root();
    let registry = load_registry(&client()?, &root, false)?;
    let entry = registry
        .packages
        .iter()
        .find(|entry| entry.name == *name)
        .ok_or_else(|| anyhow!("unknown package '{name}'"))?;
    println!("{name}");
    for dependency in &entry.requires {
        println!("  Depends: {dependency}");
    }
    Ok(())
}

fn cmd_clean(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: bpt clean");
    }
    let cache = bos_root().join(CACHE_DIR);
    match fs::remove_dir_all(&cache) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        result => result?,
    }
    println!("bpt: registry cache cleared");
    Ok(())
}

fn cmd_download(args: &[String]) -> Result<()> {
    let [spec] = args else {
        bail!("usage: bpt download <package|owner/repo|url>");
    };
    let root = bos_root();
    let client = client()?;
    let entry = if spec.contains('/') {
        parse_direct(spec, None, None)?
    } else {
        load_registry(&client, &root, false)?
            .packages
            .into_iter()
            .find(|entry| entry.name == *spec)
            .ok_or_else(|| anyhow!("unknown package '{spec}'"))?
    };
    let archive = fetch_package(&client, &entry)?;
    let path = format!("{}.tar.gz", entry.name);
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    if let Err(error) = out.write_all(&archive) {
        drop(out);
        let _ = fs::remove_file(&path);
        return Err(error.into());
    }
    println!("bpt: downloaded {path}");
    Ok(())
}

fn validate_name(name: &str) -> Result<()> {
    if name.len() > 120
        || !name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        bail!("invalid package name '{name}'");
    }
    Ok(())
}

fn validate_relative(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains(['\\', '\0'])
        || !Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        bail!("invalid repository path '{path}'");
    }
    Ok(())
}

fn validate_entry(entry: &PackageEntry) -> Result<()> {
    validate_name(&entry.name)?;
    let Some((owner, repo)) = entry.repo.split_once('/') else {
        bail!("expected a GitHub owner/repo");
    };
    validate_name(owner)?;
    validate_name(repo)?;
    if entry.git_ref.is_empty()
        || entry.git_ref.contains(['\\', '\0'])
        || entry
            .git_ref
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        bail!("invalid Git ref '{}'", entry.git_ref);
    }
    if entry.path != "." {
        validate_relative(&entry.path)?;
    }
    for bin in &entry.bin {
        validate_relative(bin)?;
    }
    for dep in &entry.requires {
        validate_name(dep)?;
    }
    Ok(())
}

fn parse_direct(
    spec: &str,
    git_ref: Option<&str>,
    path_override: Option<&str>,
) -> Result<PackageEntry> {
    let (repo, specified_ref, specified_path) = if spec.contains("://") {
        let url = url::Url::parse(spec)?;
        if url.scheme() != "https"
            || url.host_str() != Some("github.com")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            bail!("expected an HTTPS URL on github.com");
        }
        let parts: Vec<String> = url
            .path()
            .trim_matches('/')
            .split('/')
            .map(|part| {
                percent_encoding::percent_decode_str(part)
                    .decode_utf8()
                    .map(|s| s.into_owned())
            })
            .collect::<Result<_, _>>()?;
        match parts.as_slice() {
            [owner, repo] => (format!("{owner}/{repo}"), None, None),
            [owner, repo, kind, reference, path @ ..]
                if matches!(kind.as_str(), "blob" | "tree") =>
            {
                if kind == "blob" && path.is_empty() {
                    bail!("a GitHub blob URL must identify a file");
                }
                (
                    format!("{owner}/{repo}"),
                    Some(reference.clone()),
                    (!path.is_empty()).then(|| path.join("/")),
                )
            }
            _ => bail!("expected a repository, blob, or tree URL"),
        }
    } else {
        let (repo_ref, path) = spec
            .split_once(':')
            .map_or((spec, None), |(repo, path)| (repo, Some(path.to_owned())));
        let (repo, reference) = repo_ref
            .split_once('@')
            .map_or((repo_ref, None), |(repo, reference)| {
                (repo, Some(reference.to_owned()))
            });
        (repo.trim_end_matches('/').to_owned(), reference, path)
    };
    let repo = repo.trim_end_matches(".git").to_owned();
    let git_ref = git_ref
        .map(str::to_owned)
        .or(specified_ref)
        .unwrap_or_else(|| "HEAD".into());
    let path = path_override
        .map(str::to_owned)
        .or(specified_path)
        .unwrap_or_else(default_path);
    let repo_name = repo.rsplit('/').next().unwrap_or_default();
    let name = if path == "." {
        repo_name.to_owned()
    } else {
        let filename = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default();
        if validate_name(filename).is_ok() {
            filename.to_owned()
        } else {
            use sha2::{Digest, Sha256};
            format!("file-{:x}", Sha256::digest(format!("{repo}/{path}")))[..21].to_owned()
        }
    };
    let entry = PackageEntry {
        name,
        version: git_ref.clone(),
        git_ref,
        path,
        description: format!("Direct install from github.com/{repo}"),
        category: "direct".into(),
        repo,
        sha256: String::new(),
        bin: Vec::new(),
        license: String::new(),
        homepage: String::new(),
        requires: Vec::new(),
    };
    validate_entry(&entry)?;
    Ok(entry)
}

struct Installer<'a> {
    root: &'a Path,
    client: &'a reqwest::blocking::Client,
    registry: &'a Registry,
    db: &'a mut InstalledDb,
    visiting: HashSet<String>,
}

impl Installer<'_> {
    fn install(
        &mut self,
        entry: PackageEntry,
        catalog: bool,
        automatic: bool,
        force: bool,
    ) -> Result<()> {
        validate_entry(&entry)?;
        if !self.visiting.insert(entry.name.clone()) {
            bail!("dependency cycle at '{}'", entry.name);
        }
        let result = self.install_entry(&entry, catalog, automatic, force);
        self.visiting.remove(&entry.name);
        result
    }

    fn install_entry(
        &mut self,
        entry: &PackageEntry,
        catalog: bool,
        automatic: bool,
        force: bool,
    ) -> Result<()> {
        let dst = self.root.join(APPS_DIR).join(&entry.name);
        let existing = self
            .db
            .packages
            .iter()
            .position(|record| record.name == entry.name);
        if let Some(index) = existing {
            let record = &self.db.packages[index];
            if record.entry.as_ref().is_some_and(|old| {
                record.catalog != catalog
                    || (!catalog && (old.repo != entry.repo || old.path != entry.path))
            }) {
                bail!(
                    "'{}' is already installed from another source; remove it first",
                    entry.name
                );
            }
            if !force && dst.is_dir() {
                if !automatic && record.automatic {
                    self.db.packages[index].automatic = false;
                    save_db(self.root, self.db)?;
                }
                println!("bpt: '{}' is already installed", entry.name);
                return Ok(());
            }
        } else if fs::symlink_metadata(&dst).is_ok() {
            bail!("{} already exists outside bPT", dst.display());
        }
        for dep in &entry.requires {
            let dependency = self
                .registry
                .packages
                .iter()
                .find(|candidate| candidate.name == *dep)
                .ok_or_else(|| anyhow!("'{}' depends on unknown package '{dep}'", entry.name))?;
            self.install(dependency.clone(), true, true, false)?;
        }

        let mut pending: Vec<_> = entry.requires.iter().map(String::as_str).collect();
        let mut seen = HashSet::new();
        while let Some(name) = pending.pop() {
            if name == entry.name {
                bail!("dependency cycle at '{}'", entry.name);
            }
            if seen.insert(name) {
                if let Some(record) = self.db.packages.iter().find(|record| record.name == name) {
                    pending.extend(record.requires.iter().map(String::as_str));
                }
            }
        }

        let archive = fetch_package(self.client, entry)?;
        let work = work_dir(self.root, &entry.name)?;
        let staged = work.join("new");
        let backup = work.join("previous");
        let result = (|| {
            fs::create_dir(&staged)?;
            extract_package(&archive, entry, &staged)?;
            make_executable(&staged, &entry.bin)?;
            let previous = self
                .db
                .packages
                .iter()
                .position(|record| record.name == entry.name)
                .map(|index| self.db.packages.remove(index));
            let automatic = automatic && previous.as_ref().is_none_or(|record| record.automatic);
            self.db.packages.push(InstalledRecord {
                name: entry.name.clone(),
                version: entry.version.clone(),
                source: format!("github:{}@{}:{}", entry.repo, entry.git_ref, entry.path),
                installed_at: iso_now(),
                automatic,
                catalog,
                requires: entry.requires.clone(),
                entry: Some(entry.clone()),
            });
            if let Err(error) = replace_package(self.root, self.db, &dst, Some(&staged), &backup) {
                self.db.packages.pop();
                if let Some(previous) = previous {
                    self.db.packages.push(previous);
                }
                return Err(error);
            }
            Ok(())
        })();
        finish_work(&work, &backup, result)?;
        log(
            self.root,
            &format!("install {} {}", entry.name, entry.version),
        );
        println!(
            "bpt: installed '{}' ({}) into applications/{}",
            entry.name, entry.version, entry.name
        );
        Ok(())
    }
}

fn work_dir(root: &Path, name: &str) -> Result<PathBuf> {
    let apps = root.join(APPS_DIR);
    fs::create_dir_all(&apps)?;
    let work = apps.join(format!(".bpt-{name}"));
    fs::create_dir(&work)
        .with_context(|| format!("creating package staging directory {}", work.display()))?;
    Ok(work)
}

fn replace_package(
    root: &Path,
    db: &InstalledDb,
    dst: &Path,
    staged: Option<&Path>,
    backup: &Path,
) -> Result<()> {
    let had_previous = fs::symlink_metadata(dst).is_ok();
    if had_previous {
        fs::rename(dst, backup)?;
    }
    let result = (|| {
        if let Some(staged) = staged {
            fs::rename(staged, dst)?;
        }
        save_db(root, db)
    })();
    if let Err(error) = result {
        if let Some(staged) = staged {
            if dst.exists() {
                fs::rename(dst, staged)
                    .with_context(|| format!("{error:#}; moving failed installation aside"))?;
            }
        }
        if had_previous {
            fs::rename(backup, dst)
                .with_context(|| format!("{error:#}; restoring previous installation"))?;
        }
        return Err(error);
    }
    Ok(())
}

fn finish_work(work: &Path, backup: &Path, result: Result<()>) -> Result<()> {
    if result.is_ok() || fs::symlink_metadata(backup).is_err() {
        fs::remove_dir_all(work).with_context(|| format!("cleaning {}", work.display()))?;
    }
    result
}

fn fetch_package(client: &reqwest::blocking::Client, entry: &PackageEntry) -> Result<Vec<u8>> {
    validate_entry(entry)?;
    let archive = download_tarball(client, entry)?;
    if !entry.sha256.trim().is_empty() {
        verify_sha256(&archive, &entry.sha256)?;
    }
    Ok(archive)
}

fn download_tarball(client: &reqwest::blocking::Client, entry: &PackageEntry) -> Result<Vec<u8>> {
    let (owner, repo) = entry.repo.split_once('/').expect("validated repo");
    let base = url::Url::parse("https://codeload.github.com/")?;
    let mut candidates = Vec::new();
    let prefixes: &[&[&str]] = if entry.git_ref == "HEAD" || entry.git_ref.starts_with("refs/") {
        &[&[]]
    } else {
        &[&["refs", "heads"], &["refs", "tags"], &[]]
    };
    for prefix in prefixes {
        let mut url = base.clone();
        url.path_segments_mut()
            .unwrap()
            .pop_if_empty()
            .extend([owner, repo, "tar.gz"])
            .extend(prefix.iter().copied())
            .extend(entry.git_ref.split('/'));
        candidates.push(url);
    }
    let mut last_err = String::new();
    for url in candidates {
        match client.get(url.clone()).send() {
            Ok(r) if r.status().is_success() => {
                const LIMIT: u64 = 256 * 1024 * 1024;
                let mut archive = Vec::new();
                r.take(LIMIT + 1)
                    .read_to_end(&mut archive)
                    .context("reading tarball")?;
                if archive.len() as u64 > LIMIT {
                    bail!("GitHub archive exceeds 256 MiB");
                }
                return Ok(archive);
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
    println!("bpt: sha256 verified");
    Ok(())
}

/// Extract only the selected path. Links and special files are never installed.
fn extract_package(tarball: &[u8], entry: &PackageEntry, dst: &Path) -> Result<()> {
    let gz = flate2::read::GzDecoder::new(tarball);
    let mut archive = tar::Archive::new(gz);
    let mut found = false;
    let mut expanded = 0u64;
    for item in archive.entries()? {
        let mut item = item?;
        let path = item.path()?.into_owned();
        let mut parts = path.components();
        if !matches!(parts.next(), Some(Component::Normal(_)))
            || !parts
                .clone()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            bail!("unsafe path in GitHub archive: {}", path.display());
        }
        let relative = parts.as_path();
        let kind = item.header().entry_type();
        let target = if entry.path == "." {
            dst.join(relative)
        } else if let Ok(suffix) = relative.strip_prefix(&entry.path) {
            if suffix.as_os_str().is_empty() && !kind.is_dir() {
                dst.join(
                    relative
                        .file_name()
                        .ok_or_else(|| anyhow!("missing filename"))?,
                )
            } else {
                dst.join(suffix)
            }
        } else {
            continue;
        };
        found = true;
        if kind.is_dir() {
            fs::create_dir_all(&target)?;
        } else if kind.is_file() {
            expanded = expanded.saturating_add(item.size());
            if expanded > 1024 * 1024 * 1024 {
                bail!("expanded package exceeds 1 GiB");
            }
            fs::create_dir_all(
                target
                    .parent()
                    .ok_or_else(|| anyhow!("missing package parent"))?,
            )?;
            item.unpack(&target)
                .with_context(|| format!("extracting {}", path.display()))?;
        } else {
            bail!("unsupported link or special file: {}", path.display());
        }
    }
    if !found {
        bail!("path '{}' not found in the package", entry.path);
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(root: &Path, bins: &[String]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for b in bins {
        let p = root.join(b);
        let meta = fs::metadata(&p).with_context(|| format!("missing executable '{b}'"))?;
        if !meta.is_file() {
            bail!("executable '{b}' must be a regular file");
        }
        let mut perm = meta.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&p, perm)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_root: &Path, _bins: &[String]) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut archive = tar::Builder::new(encoder);
        for (name, contents) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, name, *contents).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn parses_a_single_file_and_rejects_unsafe_sources() {
        let entry = parse_direct(
            "https://github.com/example/repo/blob/v1/path/my%20file.txt",
            None,
            None,
        )
        .unwrap();
        assert_eq!(entry.repo, "example/repo");
        assert_eq!(entry.git_ref, "v1");
        assert_eq!(entry.path, "path/my file.txt");
        assert!(parse_direct(
            "https://notgithub.com/example/repo/blob/main/file",
            None,
            None
        )
        .is_err());
        assert!(parse_direct("example/repo:../../file", None, None).is_err());
        assert!(parse_direct("example/repo:/etc/passwd", None, None).is_err());
    }

    #[test]
    fn extracts_only_the_selected_file() {
        let entry = parse_direct("example/repo@main:docs/readme.txt", None, None).unwrap();
        let dest =
            std::env::temp_dir().join(format!("bpt-single-file-test-{}", std::process::id()));
        fs::create_dir(&dest).unwrap();
        let archive = archive(&[
            ("repo-main/docs/readme.txt", b"hello"),
            ("repo-main/unrelated", b"ignored"),
        ]);
        extract_package(&archive, &entry, &dest).unwrap();
        assert_eq!(fs::read(dest.join("readme.txt")).unwrap(), b"hello");
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
        fs::remove_dir_all(dest).unwrap();
    }

    #[test]
    fn rolls_back_if_database_write_fails() {
        let root = std::env::temp_dir().join(format!("bpt-rollback-test-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let dst = root.join("app");
        let staged = root.join("stage");
        let backup = root.join("previous");
        fs::create_dir(&dst).unwrap();
        fs::write(dst.join("file"), "old").unwrap();
        fs::create_dir(&staged).unwrap();
        fs::write(staged.join("file"), "new").unwrap();
        assert!(
            replace_package(&root, &InstalledDb::default(), &dst, Some(&staged), &backup).is_err()
        );
        assert_eq!(fs::read_to_string(dst.join("file")).unwrap(), "old");
        assert!(!backup.exists());
        fs::remove_dir_all(root).unwrap();
    }
}

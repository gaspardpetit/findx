use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "localindex", version, about = "Local document indexer")]
pub struct Cli {
    #[arg(
        long,
        global = true,
        value_name = "FILE",
        default_value = "localindex.toml"
    )]
    pub config: Utf8PathBuf,

    #[arg(long, global = true, value_enum, default_value = "text")]
    pub log_format: LogFormat,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Index(IndexArgs),
    Watch(WatchArgs),
    Query(QueryArgs),
    Oneshot(OneshotArgs),
    Serve(ServeArgs),
    Migrate(MigrateArgs),
    Status,
}

#[derive(Args, Debug, Default)]
pub struct IndexArgs {
    #[arg(long, value_delimiter = ',', value_name = "PATHS")]
    pub roots: Vec<Utf8PathBuf>,

    #[arg(long, value_name = "FILE")]
    pub db: Option<Utf8PathBuf>,

    #[arg(long, value_name = "DIR", name = "tantivy-index")]
    pub tantivy_index: Option<Utf8PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct WatchArgs {
    #[command(flatten)]
    pub index: IndexArgs,

    #[arg(long, value_name = "N", default_value_t = num_cpus::get())]
    pub threads: usize,
}

#[derive(Args, Debug, Default)]
pub struct QueryArgs {
    #[arg(long, value_name = "FILE")]
    pub db: Option<Utf8PathBuf>,

    #[arg(long, value_name = "DIR", name = "tantivy-index")]
    pub tantivy_index: Option<Utf8PathBuf>,

    #[arg(long, value_enum, default_value = "keyword")]
    pub mode: QueryMode,

    #[arg(long, default_value_t = 20)]
    pub top_k: usize,

    #[arg(value_name = "QUERY")]
    pub query: String,

    #[arg(long, default_value_t = false)]
    pub chunks: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum QueryMode {
    Keyword,
    Semantic,
    Hybrid,
}

impl Default for QueryMode {
    fn default() -> Self {
        QueryMode::Keyword
    }
}

#[derive(Args, Debug, Default)]
pub struct OneshotArgs {
    #[command(flatten)]
    pub index: IndexArgs,

    #[command(flatten)]
    pub query: QueryArgs,
}

#[derive(Args, Debug, Default)]
pub struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub bind: String,
}

#[derive(Args, Debug, Default)]
pub struct MigrateArgs {
    #[arg(long)]
    pub check: bool,

    #[arg(long)]
    pub apply: bool,
}

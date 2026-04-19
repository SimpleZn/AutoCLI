pub mod ai_generate;
pub mod cascade;
pub mod config;
pub mod cookiecloud;
pub mod explore;
pub mod generate;
pub mod llm;
pub mod synthesize;
pub mod types;
pub mod url_pattern;

pub use ai_generate::generate_with_ai;
pub use cascade::{cascade, probe_endpoint, render_cascade_result, CascadeResult};
pub use config::{
    api_base, command_config_url, config_path, load_config, save_config, search_url, upload_url,
    user_agent, Config, CookieCloudConfig, LlmConfig,
};
pub use cookiecloud::{fetch_all_cookies, fetch_cookies_for_domain};
pub use explore::explore;
pub use generate::{
    generate, generate_full, normalize_goal, render_generate_summary, GenerateExploreStats,
    GenerateOptions, GenerateResult, GenerateSynthesizeStats,
};
pub use synthesize::{
    render_synthesize_summary, synthesize, SynthesizeCandidateSummary, SynthesizeResult,
};
pub use types::{
    AdapterCandidate, DiscoveredEndpoint, ExploreManifest, ExploreOptions, ExploreResult,
    FieldInfo, InferredCapability, RecommendedArg, ResponseAnalysis, StoreHint, StoreInfo,
    StrategyTestResult, SynthesizeOptions,
};
pub use url_pattern::url_to_pattern;

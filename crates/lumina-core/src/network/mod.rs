mod reqwest_resolver;
mod resolver;

pub use reqwest_resolver::ReqwestResolver;
pub use resolver::{AppResolver, ResolveResult, ResolverMode, ResolverPolicy};

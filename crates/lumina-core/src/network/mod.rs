mod resolver;
mod reqwest_resolver;

pub use resolver::{AppResolver, ResolveResult, ResolverMode, ResolverPolicy};
pub use reqwest_resolver::ReqwestResolver;

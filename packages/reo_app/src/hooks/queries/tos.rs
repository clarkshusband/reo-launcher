use freya::query::{Query, QueryCapability, UseQuery, use_query};
use reo_core::{LauncherError, TermsDocument, fetch_terms};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TermsKeys {
    pub meta_url_base: String,
    /// Startup mounts this before the launcher is up and a settled error is
    /// never retried, so readiness is part of the key to run it again
    pub ready: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TermsQuery;

impl QueryCapability for TermsQuery {
    type Ok = TermsDocument;
    type Err = LauncherError;
    type Keys = TermsKeys;

    async fn run(&self, _keys: &Self::Keys) -> Result<Self::Ok, Self::Err> {
        let state = crate::launcher::state()?;
        fetch_terms(&state.services.requester).await
    }
}

pub fn use_terms() -> UseQuery<TermsQuery> {
    let meta_url_base = super::use_meta_url_key();
    let ready = crate::hooks::use_launcher().ready;

    use_query(Query::new(
        TermsKeys {
            meta_url_base,
            ready,
        },
        TermsQuery,
    ))
}

pub fn terms_document(query: &UseQuery<TermsQuery>) -> Option<TermsDocument> {
    super::state::settled_or_loading(query)
}

pub fn terms_error(query: &UseQuery<TermsQuery>) -> Option<String> {
    super::state::query_error(query)
}

pub fn terms_is_loading(query: &UseQuery<TermsQuery>) -> bool {
    super::state::query_is_loading(query)
}

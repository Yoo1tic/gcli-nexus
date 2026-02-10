use url::Url;

fn build_provider_url(base: &Url, path: &str, query: Option<&str>) -> Url {
    let mut url = base.clone();
    url.set_path(path);
    url.set_query(query);
    url
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderEndpoints {
    stream: Url,
    no_stream: Url,
}

impl ProviderEndpoints {
    pub(crate) fn new(
        base: Url,
        stream_path: &str,
        stream_query: Option<&str>,
        no_stream_path: &str,
        no_stream_query: Option<&str>,
    ) -> Self {
        Self {
            stream: build_provider_url(&base, stream_path, stream_query),
            no_stream: build_provider_url(&base, no_stream_path, no_stream_query),
        }
    }

    pub(crate) fn select(&self, stream: bool) -> &Url {
        if stream {
            &self.stream
        } else {
            &self.no_stream
        }
    }
}

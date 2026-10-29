//! Node-shaped JSON view of [`Config`] / [`Provider`] (camelCase keys, unset fields absent).
//! Used by the golden tests and by callers that need the plain config object.

use crate::config::Config;
use crate::provider::{ApiKey, ModelMeta, Protocol, Provider, TypeSource};
use serde_json::{Map, Value, json};

fn num(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

impl ModelMeta {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        let opt_s = |s: &Option<String>| s.clone().map_or(Value::Null, Value::String);
        m.insert("name".into(), opt_s(&self.name));
        m.insert(
            "contextWindow".into(),
            self.context_window.map_or(Value::Null, num),
        );
        m.insert("maxOutput".into(), self.max_output.map_or(Value::Null, num));
        m.insert("vendor".into(), opt_s(&self.vendor));
        m.insert("family".into(), opt_s(&self.family));
        for (k, v) in [
            ("quality", &self.quality),
            ("qualityScore", &self.quality_score),
            ("qualityRank", &self.quality_rank),
            ("reasoning", &self.reasoning),
            ("thinking", &self.thinking),
        ] {
            if let Some(v) = v {
                m.insert(k.into(), v.clone());
            }
        }
        m.insert(
            "capabilities".into(),
            Value::Object(self.capabilities.clone()),
        );
        Value::Object(m)
    }
}

impl Provider {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("type".into(), self.protocol.as_str().into());
        m.insert("typeSource".into(), self.type_source.as_str().into());
        let mut put = |k: &str, v: Option<Value>| {
            if let Some(v) = v {
                m.insert(k.into(), v);
            }
        };
        let s = |o: &Option<String>| o.clone().map(Value::String);
        put("npm", s(&self.npm));
        put("name", s(&self.name));
        put("baseURL", s(&self.base_url));
        put(
            "apiKey",
            self.api_key.as_ref().map(|k| match k {
                ApiKey::Disabled => Value::Bool(false),
                ApiKey::Value(v) => Value::String(v.clone()),
            }),
        );
        put("apiKeySource", s(&self.api_key_source));
        put("authToken", s(&self.auth_token));
        put("authTokenSource", s(&self.auth_token_source));
        put("authHeader", s(&self.auth_header));
        put(
            "headers",
            self.headers.as_ref().map(|h| {
                Value::Object(
                    h.iter()
                        .map(|(k, v)| (k.clone(), Value::from(v.clone())))
                        .collect(),
                )
            }),
        );
        put("models", self.models.as_ref().map(|m| json!(m)));
        put(
            "modelsMeta",
            self.models_meta.as_ref().map(|mm| {
                Value::Object(mm.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
            }),
        );
        put("discover", self.discover.map(Value::Bool));
        put("noAuth", self.no_auth.map(Value::Bool));
        Value::Object(m)
    }

    /// Lenient inverse of [`Provider::to_json`] (missing `type` defaults to anthropic).
    pub fn from_json(v: &Value) -> Provider {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        Provider {
            protocol: if s("type").as_deref() == Some("openai") {
                Protocol::OpenAi
            } else {
                Protocol::Anthropic
            },
            type_source: match s("typeSource").as_deref() {
                Some("explicit") => TypeSource::Explicit,
                Some("npm") => TypeSource::Npm,
                Some("url") => TypeSource::Url,
                Some("key") => TypeSource::Key,
                Some("default") => TypeSource::Default,
                _ => TypeSource::Id,
            },
            npm: s("npm"),
            name: s("name"),
            base_url: s("baseURL"),
            api_key: match v.get("apiKey") {
                Some(Value::Bool(false)) => Some(ApiKey::Disabled),
                Some(Value::String(k)) => Some(ApiKey::Value(k.clone())),
                _ => None,
            },
            api_key_source: s("apiKeySource"),
            auth_token: s("authToken"),
            auth_token_source: s("authTokenSource"),
            auth_header: s("authHeader"),
            headers: v.get("headers").and_then(Value::as_object).map(|h| {
                h.iter()
                    .filter_map(|(k, x)| x.as_str().map(|x| (k.clone(), x.to_string())))
                    .collect()
            }),
            models: v.get("models").and_then(Value::as_array).map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            }),
            models_meta: None,
            discover: v.get("discover").and_then(Value::as_bool),
            no_auth: v.get("noAuth").and_then(Value::as_bool),
        }
    }
}

impl Config {
    /// Node's merged config object, including the `_modelExplicit` / `_explicitAliases` /
    /// `_warnings` metadata.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        if let Some(s) = &self.schema {
            m.insert("$schema".into(), s.clone().into());
        }
        m.insert("_explicitAliases".into(), json!(self.explicit_aliases));
        m.insert("_modelExplicit".into(), self.model_explicit.into());
        m.insert("_warnings".into(), json!(self.warnings));
        m.insert(
            "aliases".into(),
            Value::Object(
                self.aliases
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone().map_or(Value::Null, Value::String)))
                    .collect(),
            ),
        );
        m.insert("maxTokens".into(), num(self.max_tokens));
        if let Some(x) = &self.model {
            m.insert("model".into(), x.clone().into());
        }
        if let Some(x) = &self.small_model {
            m.insert("smallModel".into(), x.clone().into());
        }
        for (k, v) in &self.extra {
            m.insert(k.clone(), v.clone());
        }
        let providers: Map<String, Value> = self
            .providers
            .iter()
            .map(|(k, p)| (k.clone(), p.to_json()))
            .collect();
        m.insert("providers".into(), Value::Object(providers));
        Value::Object(m)
    }

    /// Lenient inverse of [`Config::to_json`] (for fixtures and tests).
    pub fn from_json(v: &Value) -> Config {
        let mut c = Config::default();
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        c.aliases = v
            .get("aliases")
            .and_then(Value::as_object)
            .map(|a| {
                a.iter()
                    .map(|(k, x)| (k.clone(), x.as_str().map(String::from)))
                    .collect()
            })
            .unwrap_or_default();
        c.providers = v
            .get("providers")
            .and_then(Value::as_object)
            .map(|p| {
                p.iter()
                    .map(|(k, x)| (k.clone(), Provider::from_json(x)))
                    .collect()
            })
            .unwrap_or_default();
        c.model = s("model");
        c.small_model = s("smallModel");
        c.schema = s("$schema");
        if let Some(t) = v.get("maxTokens").and_then(Value::as_f64) {
            c.max_tokens = t;
        }
        c
    }
}

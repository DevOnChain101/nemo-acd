use std::path::Path;

/// Names whose content must not be copied into snapshots or prompts: dotenv files (not
/// `.example`/`.sample`/`.template`), private keys, certificates/keystores, cloud and
/// package-manager credential files. Name based, case-insensitive.
pub fn is_secret_path(path: &Path) -> bool {
    let Some(n) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let n = n.to_ascii_lowercase();
    if n == ".env" || n.starts_with(".env.") {
        return !(n.ends_with(".example") || n.ends_with(".sample") || n.ends_with(".template"));
    }
    ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519", "credentials"]
        .iter()
        .any(|p| n.starts_with(p))
        || [".pem", ".key", ".p12", ".pfx", ".keystore", ".jks"]
            .iter()
            .any(|x| n.ends_with(x))
        || [".netrc", "_netrc", ".npmrc", ".pgpass", ".git-credentials"].contains(&n.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        for (p, want) in [
            (".env", true),
            (".ENV.local", true),
            ("a/b/.env.production", true),
            (".env.example", false),
            (".env.sample", false),
            (".env.template", false),
            ("environment.rs", false),
            ("id_rsa", true),
            ("id_rsa.pub", true),
            ("id_ed25519", true),
            ("credentials", true),
            ("credentials.json", true),
            ("server.PEM", true),
            ("tls.key", true),
            ("a.p12", true),
            ("a.pfx", true),
            ("a.keystore", true),
            ("a.jks", true),
            (".netrc", true),
            ("_netrc", true),
            (".npmrc", true),
            (".pgpass", true),
            (".git-credentials", true),
            ("README.md", false),
            ("keys.txt", false),
            ("src/main.rs", false),
            ("", false),
            ("/", false),
        ] {
            assert_eq!(is_secret_path(Path::new(p)), want, "{p}");
        }
    }
}

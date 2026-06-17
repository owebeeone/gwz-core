    use std::fs;
    use std::path::Path;
    
    
    use std::time::{SystemTime, UNIX_EPOCH};

    
    
    
    

    
use super::*;

impl TempDir {
        pub(crate) fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "gwz-core-ops-{prefix}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }
    }

    
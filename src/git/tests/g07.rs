    use std::fs;
    
    

    

    
use super::*;

impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

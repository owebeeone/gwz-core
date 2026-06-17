    
    
    

    

    
use super::*;

#[test]
    pub(crate) fn reports_commit_ancestry_without_moving_head() {
        let temp = TempDir::new("ancestry");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repo");
        backend.create_repo(&repo_path).unwrap();
        let first = commit_file(&repo_path, "README.md", "one", "initial", &[]).unwrap();
        let first_oid = git2::Oid::from_str(&first).unwrap();
        let second = commit_file(&repo_path, "README.md", "two", "second", &[first_oid]).unwrap();

        assert!(backend.is_ancestor(&repo_path, &first, &second).unwrap());
        assert!(!backend.is_ancestor(&repo_path, &second, &first).unwrap());
        assert_eq!(backend.head(&repo_path).unwrap().commit, Some(second));
    }

    
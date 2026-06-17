    
    
    
    
    

    
    
    
    

    
use super::*;

#[test]
    pub(crate) fn init_from_sources_derives_default_paths_from_windows_local_paths() {
        let manifest = crate::artifact::ManifestArtifact {
            schema: crate::artifact::WORKSPACE_SCHEMA.to_owned(),
            workspace: crate::artifact::WorkspaceHeader {
                id: "ws_ops".to_owned(),
            },
            members: Vec::new(),
        };

        let plans = init_source_plans(
            &manifest,
            &[crate::SourceUrl {
                url: r"C:\Users\runneradmin\AppData\Local\Temp\remote.git".to_owned(),
                path: None,
                remote_name: None,
                branch: None,
            }],
        )
        .unwrap();

        assert_eq!(plans[0].path.as_str(), "remote");
        assert_eq!(plans[0].member_id, "mem_remote");
        assert_eq!(plans[0].source_id, "src_remote");
    }

    
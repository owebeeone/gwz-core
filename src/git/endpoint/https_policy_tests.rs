use super::*;
#[test]
fn status_policy_never_replays_posts_or_network_failures() {
    for status in 100..=599 {
        let action = classify(status, GitService::ReceivePackExchange, true);
        assert!(!matches!(
            action,
            ResponseAction::Redirect | ResponseAction::Authenticate
        ));
        if status == 200 {
            assert_eq!(action, ResponseAction::Success);
        }
    }
    assert_eq!(
        classify(401, GitService::UploadPackAdvertisement, true),
        ResponseAction::Authenticate
    );
    assert_eq!(
        classify(404, GitService::UploadPackAdvertisement, true),
        ResponseAction::Authenticate
    );
    assert_eq!(
        classify(404, GitService::UploadPackAdvertisement, false),
        ResponseAction::Fail(ErrorCode::RepositoryRefused)
    );
    assert_eq!(
        classify(403, GitService::ReceivePackExchange, false),
        ResponseAction::Fail(ErrorCode::Io)
    );
    for status in [204, 206, 101, 199, 600] {
        assert_eq!(
            classify(status, GitService::UploadPackAdvertisement, false),
            ResponseAction::Fail(ErrorCode::Protocol)
        );
    }
}
#[test]
fn competing_redirects_cannot_overwrite_or_retire_an_operation_route() {
    let mut routes = Routes::new(2);
    let key = RouteKey::new(
        "op",
        "https://original/repo",
        GitService::UploadPackAdvertisement,
    );
    routes.admit(key.clone()).unwrap();
    routes.install(&key, "https://first/repo").unwrap();
    assert_eq!(
        routes.install(&key, "https://other/repo"),
        Err(ErrorCode::Protocol)
    );
    assert_eq!(routes.get(&key).unwrap(), "https://first/repo");
    routes.admit(key.clone()).unwrap();
    routes.install(&key, "https://first/repo").unwrap();
    routes
        .admit(RouteKey::new(
            "op",
            "https://second/repo",
            GitService::UploadPackAdvertisement,
        ))
        .unwrap();
    assert_eq!(
        routes.admit(RouteKey::new(
            "op",
            "https://third/repo",
            GitService::UploadPackAdvertisement
        )),
        Err(ErrorCode::Capacity)
    );
    routes.finish("op");
    assert!(routes.get(&key).is_err());
}

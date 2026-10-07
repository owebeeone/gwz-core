use super::*;
#[test]
fn status_policy_never_replays_posts_or_network_failures() {
    for status in 100..=599 {
        let action = classify(status, GitService::ReceivePackExchange);
        assert!(!matches!(action, ResponseAction::Redirect));
        if status == 200 {
            assert_eq!(action, ResponseAction::Success);
        }
    }
    assert_eq!(
        classify(401, GitService::UploadPackAdvertisement),
        ResponseAction::Fail(ErrorCode::Authentication)
    );
    assert_eq!(
        classify(404, GitService::UploadPackAdvertisement),
        ResponseAction::Fail(ErrorCode::RepositoryRefused)
    );
    assert_eq!(
        classify(403, GitService::ReceivePackExchange),
        ResponseAction::Fail(ErrorCode::Io)
    );
    for status in [204, 206, 101, 199, 600] {
        assert_eq!(
            classify(status, GitService::UploadPackAdvertisement),
            ResponseAction::Fail(ErrorCode::Protocol)
        );
    }
}
#[test]
fn competing_redirects_cannot_overwrite_or_retire_an_operation_route() {
    let mut routes = Routes::new();
    let key = RouteKey::new(
        "op",
        "https://original/repo",
        GitService::UploadPackAdvertisement,
    );
    routes.admit(key.clone());
    routes.install(&key, "https://first/repo").unwrap();
    assert_eq!(
        routes.install(&key, "https://other/repo"),
        Err(ErrorCode::Protocol)
    );
    assert_eq!(routes.get(&key).unwrap(), "https://first/repo");
    routes.admit(key.clone());
    routes.install(&key, "https://first/repo").unwrap();
    routes.admit(RouteKey::new(
        "op",
        "https://second/repo",
        GitService::UploadPackAdvertisement,
    ));
    routes.finish("op");
    assert!(routes.get(&key).is_err());
}
#[test]
fn a_route_table_admits_every_distinct_url_a_request_names_and_clears_with_the_operation() {
    // The table once refused its 65th route with `Capacity`. A route is a URL
    // key and a few small values, and nothing frees one before its operation
    // ends (the remote that discovered it is gone before the exchange asks
    // for it), so a count would only fail a repository that the server allows.
    let mut routes = Routes::new();
    for index in 0..200 {
        let key = RouteKey::new(
            "op",
            &format!("https://host/repo-{index}"),
            GitService::UploadPackAdvertisement,
        );
        routes.admit(key.clone());
        routes
            .install(&key, &format!("https://final/repo-{index}"))
            .unwrap();
    }
    assert_eq!(routes.len(), 200);
    routes.finish("op");
    assert_eq!(routes.len(), 0);
}

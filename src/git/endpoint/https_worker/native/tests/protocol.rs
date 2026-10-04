//! Header storage and codec, offer selection, and mechanism history.
use super::*;

cfg_if::cfg_if! { if #[cfg(all(test, unix))] {
    #[test]
    fn storage_wipes_before_deallocation_on_encoding_decode_parse_and_drop() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let probe = audit::Probe(Some(records.clone()));
        assert!(Header::encode(b"NTLM ", b"synthetic", Header::zeroed(6, probe.clone())).is_err());
        assert!(decode_owned(b"YR==", 2, probe.clone()).is_err());
        let mut headers = HeaderMap::new(); headers.insert(WWW_AUTHENTICATE, "NTLM YQ==, NTLM Yg==".parse().unwrap());
        assert!(Offers::parse_owned(&headers, probe.clone()).is_err());
        drop(Header::copy(b"synthetic", probe));
        let mut publication = https_auth::SecretHeader::from_bytes(b"synthetic");
        publication.observe_wipe(records.clone()); drop(publication);
        let records = records.lock().unwrap(); assert_eq!(records.len(), 5);
        assert!(records.iter().all(|(length, zero)| *length > 0 && *zero));
    }
    #[test]
    fn bounded_initial_token_refuses_before_native_begin_and_publication() {
        let mut headers = HeaderMap::new();
        headers.insert(WWW_AUTHENTICATE, "Negotiate c3ludGhldGlj".parse().unwrap());
        let offers = Offers::parse(&headers).unwrap();
        let selected = offers.select(false).unwrap();
        assert_eq!(selected.scheme, NativeScheme::Negotiate);
        let facts = History::new(NativeSource::CurrentLogon, selected.scheme).facts;
        assert_eq!(facts.native.as_ref().unwrap().observation, NativeObservation::NotStarted);
        assert!(!facts.credential_offered);
        assert_eq!(selected.initial().unwrap_err(), ErrorCode::UnsupportedOperation);
    }
    #[test]
    fn source_order_and_challenge_decode_are_closed_and_bounded() {
        let mut headers = HeaderMap::new();
        headers.insert(WWW_AUTHENTICATE, "Basic realm=\"x\", NTLM, Negotiate".parse().unwrap());
        let offers = Offers::parse(&headers).unwrap();
        assert!(offers.helper_allowed());
        assert_eq!(offers.select(true).unwrap().scheme, NativeScheme::Negotiate);
        assert_eq!(offers.select(false).unwrap().scheme, NativeScheme::Negotiate);
        assert_eq!(decode_challenge(b"YQ==", 1).unwrap().as_bytes(), b"a");
        assert!(decode_challenge(b"YQ==", 0).is_err());
        for invalid in [b"YQ".as_slice(), b"YR==", b" YQ==", b"YQ== ", b""] {
            assert!(decode_challenge(invalid, 8).is_err());
        }
    }
    #[test]
    fn authority_continue_is_preserved_but_does_not_authorize_remote_success() {
        let mut history = History::new(NativeSource::CurrentLogon, NativeScheme::Ntlm);
        let step = gwz_sspi::TokenStep {
            status: gwz_sspi::TokenStatus::Continue, attributes: 0,
            observation: gwz_sspi::MechanismObservation::Selected { mechanism: gwz_sspi::Mechanism::Ntlm, authoritative: true },
            payload: gwz_sspi::SecretBytes::new(b"synthetic"),
        };
        history.observe(&step).unwrap();
        assert!(history.facts.native.as_ref().unwrap().authoritative);
        assert_eq!(history.accept(200), Err(ErrorCode::Protocol));
        assert_eq!(history.facts.authenticated, None);
        history.facts.credential_offered = true;
        history.reject(401);
        assert_eq!(history.facts.authenticated, Some(false));
    }
    #[test]
    fn direct_ntlm_observation_cannot_be_unresolved_or_kerberos() {
        for observation in [gwz_sspi::MechanismObservation::Unresolved,
            gwz_sspi::MechanismObservation::Selected { mechanism: gwz_sspi::Mechanism::Kerberos, authoritative: true }] {
            let mut history = History::new(NativeSource::CurrentLogon, NativeScheme::Ntlm);
            let step = gwz_sspi::TokenStep { status: gwz_sspi::TokenStatus::Continue, attributes: 0,
                observation, payload: gwz_sspi::SecretBytes::new(b"synthetic") };
            assert_eq!(history.observe(&step), Err(ErrorCode::Protocol));
        }
    }
    #[test]
    fn provisional_resolution_cannot_switch_or_become_unresolved() {
        for observation in [gwz_sspi::MechanismObservation::Unresolved,
            gwz_sspi::MechanismObservation::Selected { mechanism: gwz_sspi::Mechanism::Kerberos, authoritative: false }] {
            let mut history = History::new(NativeSource::CurrentLogon, NativeScheme::Negotiate);
            let mut step = gwz_sspi::TokenStep { status: gwz_sspi::TokenStatus::Continue, attributes: 0,
                observation: gwz_sspi::MechanismObservation::Selected { mechanism: gwz_sspi::Mechanism::Ntlm, authoritative: false },
                payload: gwz_sspi::SecretBytes::new(b"synthetic") };
            history.observe(&step).unwrap(); step.observation = observation;
            assert_eq!(history.observe(&step), Err(ErrorCode::Protocol));
        }
    }
    #[test]
    fn header_cap_accounts_for_scheme_and_base64_before_native_work() {
        assert_eq!(raw_limit(18, NativeScheme::Ntlm).unwrap().raw_bytes(), 9);
        assert!(raw_limit(8, NativeScheme::Negotiate).is_err());
        let header = Header::token(NativeScheme::Ntlm, b"synthetic").unwrap();
        assert_eq!(header.bytes(), b"NTLM c3ludGhldGlj");
    }
} }

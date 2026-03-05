#[cfg(feature = "rkyv")]
mod rkyv_enabled {
    use crate::AppData;
    use crate::AppDataResponse;
    use crate::OptionalSerde;

    #[derive(Clone, Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    #[derive(rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
    #[derive(derive_more::Display)]
    struct RkyvEnabled {
        i: u32,
    }

    fn roundtrip(v: &RkyvEnabled) -> RkyvEnabled {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(v).unwrap();
        rkyv::from_bytes::<RkyvEnabled, rkyv::rancor::Error>(&bytes).unwrap()
    }

    #[test]
    fn test_optional_rkyv_enabled() {
        fn accept_optional(v: impl OptionalSerde) {
            let _ = v;
        }

        let v = RkyvEnabled { i: 3 };
        accept_optional(v.clone());
        assert_eq!(v, roundtrip(&v));
    }

    #[test]
    fn test_app_data_rkyv_enabled() {
        fn accept_app_data(v: impl AppData) {
            let _ = v;
        }

        let v = RkyvEnabled { i: 3 };
        accept_app_data(v.clone());
        assert_eq!(v, roundtrip(&v));
    }

    #[test]
    fn test_app_data_response_rkyv_enabled() {
        fn accept_app_data_response(v: impl AppDataResponse) {
            let _ = v;
        }

        let v = RkyvEnabled { i: 3 };
        accept_app_data_response(v.clone());
        assert_eq!(v, roundtrip(&v));
    }
}

use super::*;

// --- test 5: each bound trips as a typed error ------------------------------

/// Byte overrun is a typed error, never a silent short body.
#[tokio::test]
async fn the_total_byte_bound_trips_as_a_typed_error() {
    let big = vec![b'x'; 32 * 1024];
    let (site, tool, _pulls) = fixture(
        vec![("/big", ok(vec![&big]))],
        FetchLimits {
            max_total_bytes: 16 * 1024,
            ..FetchLimits::default()
        },
    )
    .await;
    let error = tool
        .fetch("https://corpus.example.org/big")
        .await
        .expect_err("over the bound");
    assert!(
        matches!(
            error,
            FetchToolError::BodyTooLarge { limit } if limit == 16 * 1024
        ),
        "{error:?}"
    );
    assert_eq!(site.served(), vec!["/big".to_owned()]);
}

/// The wall-clock bound trips as a typed error when the body stalls past it.
#[tokio::test]
async fn the_wall_clock_bound_trips_as_a_typed_error() {
    let (site, tool, _pulls) = fixture(
        vec![(
            "/slow",
            slow_ok(vec![b"first ", b"second"], Duration::from_millis(600)),
        )],
        FetchLimits {
            time_budget: Duration::from_millis(150),
            ..FetchLimits::default()
        },
    )
    .await;
    let error = tool
        .fetch("https://corpus.example.org/slow")
        .await
        .expect_err("stalls past the deadline");
    assert!(
        matches!(error, FetchToolError::DeadlineExceeded { .. }),
        "{error:?}"
    );
    assert_eq!(site.served(), vec!["/slow".to_owned()]);
}

/// The redirect-hop bound trips as a typed error on a redirect loop.
#[tokio::test]
async fn the_redirect_hop_bound_trips_as_a_typed_error() {
    let routes: Vec<(&'static str, CannedResponse)> = (1..=6)
        .map(|hop| {
            let next = if hop == 6 { 1 } else { hop + 1 };
            (
                match hop {
                    1 => "/r1",
                    2 => "/r2",
                    3 => "/r3",
                    4 => "/r4",
                    5 => "/r5",
                    _ => "/r6",
                },
                redirect(&format!("/r{next}")),
            )
        })
        .collect();
    let (site, tool, _pulls) = fixture(routes, FetchLimits::default()).await;
    let error = tool
        .fetch("https://corpus.example.org/r1")
        .await
        .expect_err("redirect loop");
    assert!(
        matches!(error, FetchToolError::TooManyRedirects { limit: 5 }),
        "{error:?}"
    );
    assert_eq!(
        site.served().len(),
        6,
        "six hops were connected before the bound tripped"
    );
}

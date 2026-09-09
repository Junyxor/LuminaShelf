use super::ProbeMetrics;

#[derive(Debug, Clone, Copy)]
pub struct ScoreWeights {
    pub reachability: f64,
    pub latency: f64,
    pub ttfb: f64,
    pub throughput: f64,
    pub stability: f64,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            reachability: 30.0,
            latency: 20.0,
            ttfb: 20.0,
            throughput: 15.0,
            stability: 15.0,
        }
    }
}

pub fn score_endpoint(
    metrics: &ProbeMetrics,
    success_streak: u32,
    failure_streak: u32,
    weights: ScoreWeights,
) -> f64 {
    let reachable = matches!(metrics.status_code, Some(200..=399));
    if !reachable {
        return 0.0;
    }

    let latency_score = decay(metrics.tcp_ms.or(metrics.dns_ms), 45.0, 900.0);
    let ttfb_score = decay(metrics.ttfb_ms, 100.0, 1800.0);
    let throughput_score = rise(metrics.throughput_mbps, 3.0, 80.0);
    let streak = (success_streak as f64 * 0.035).min(0.18);
    let penalty = (failure_streak as f64 * 0.16).min(0.85);
    let stability_score = (0.82 + streak - penalty).clamp(0.0, 1.0);

    weights.reachability
        + weights.latency * latency_score
        + weights.ttfb * ttfb_score
        + weights.throughput * throughput_score
        + weights.stability * stability_score
}

fn decay(value: Option<f64>, excellent: f64, bad: f64) -> f64 {
    let Some(value) = value else { return 0.25 };
    if value <= excellent {
        1.0
    } else if value >= bad {
        0.0
    } else {
        1.0 - ((value - excellent) / (bad - excellent))
    }
}

fn rise(value: Option<f64>, poor: f64, excellent: f64) -> f64 {
    let Some(value) = value else { return 0.2 };
    if value <= poor {
        0.0
    } else if value >= excellent {
        1.0
    } else {
        (value - poor) / (excellent - poor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_endpoint_scores_higher() {
        let fast = ProbeMetrics {
            tcp_ms: Some(22.0),
            ttfb_ms: Some(65.0),
            throughput_mbps: Some(75.0),
            status_code: Some(200),
            ..Default::default()
        };
        let slow = ProbeMetrics {
            tcp_ms: Some(450.0),
            ttfb_ms: Some(950.0),
            throughput_mbps: Some(8.0),
            status_code: Some(200),
            ..Default::default()
        };
        assert!(
            score_endpoint(&fast, 8, 0, ScoreWeights::default())
                > score_endpoint(&slow, 8, 0, ScoreWeights::default())
        );
    }

    #[test]
    fn unreachable_scores_zero() {
        let metrics = ProbeMetrics {
            status_code: Some(503),
            ..Default::default()
        };
        assert_eq!(score_endpoint(&metrics, 0, 0, ScoreWeights::default()), 0.0);
    }
}

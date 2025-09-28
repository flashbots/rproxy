use std::{fmt::Debug, hash::Hash, sync::Arc};

use parking_lot::RwLock;
use prometheus_client::{
    encoding::{EncodeLabelSet, EncodeMetric, MetricEncoder},
    metrics::{MetricType, TypedMetric, gauge::Gauge},
};

// CandlestickLabel ----------------------------------------------------

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq, EncodeLabelSet)]
struct CandlestickLabel {
    r#type: &'static str,
}

impl CandlestickLabel {
    const OPEN: Self = Self { r#type: "open" };
    const CLOSE: Self = Self { r#type: "close" };
    const HIGH: Self = Self { r#type: "high" };
    const LOW: Self = Self { r#type: "low" };
    const VOLUME: Self = Self { r#type: "volume" };
}

// CandlestickInner ----------------------------------------------------

#[derive(Clone)]
struct CandlestickInner {
    open: i64,
    close: i64,
    high: i64,
    low: i64,
    volume: i64,
}

impl CandlestickInner {
    fn record(&mut self, v: i64) {
        self.close += v; // it will be averaged out on the next encode

        if self.volume == 0 || v > self.high {
            self.high = v;
        }

        if self.volume == 0 || v < self.low {
            self.low = v;
        }

        self.volume += 1;
    }

    fn observe(&mut self) -> CandlestickInner {
        if self.volume > 1 {
            self.close /= self.volume;
        }

        let res = self.clone();

        self.open = res.close;
        self.close = 0;
        self.high = 0;
        self.low = 0;
        self.volume = 0;

        res
    }
}

// Candlestick ---------------------------------------------------------

#[derive(Clone)]
pub(crate) struct Candlestick {
    inner: Arc<RwLock<CandlestickInner>>,

    open: Gauge,
    close: Gauge,
    high: Gauge,
    low: Gauge,
    volume: Gauge,
}

impl Candlestick {
    pub(crate) fn record(&self, value: i64) {
        self.inner.write().record(value);
    }
}

impl TypedMetric for Candlestick {
    const TYPE: MetricType = MetricType::Gauge;
}

impl EncodeMetric for Candlestick {
    fn metric_type(&self) -> MetricType {
        MetricType::Gauge
    }

    fn encode(&self, mut encoder: MetricEncoder) -> Result<(), std::fmt::Error> {
        let this = self.inner.write().observe();

        self.open.set(this.open);
        self.close.set(this.close);
        self.high.set(this.high);
        self.low.set(this.low);
        self.volume.set(this.volume);

        self.open.encode(encoder.encode_family(&CandlestickLabel::OPEN)?)?;
        self.close.encode(encoder.encode_family(&CandlestickLabel::CLOSE)?)?;
        self.high.encode(encoder.encode_family(&CandlestickLabel::HIGH)?)?;
        self.low.encode(encoder.encode_family(&CandlestickLabel::LOW)?)?;
        self.volume.encode(encoder.encode_family(&CandlestickLabel::VOLUME)?)?;

        Ok(())
    }
}

impl Default for Candlestick {
    fn default() -> Self {
        Self {
            open: Gauge::default(),
            close: Gauge::default(),
            high: Gauge::default(),
            low: Gauge::default(),
            volume: Gauge::default(),

            inner: Arc::new(RwLock::new(CandlestickInner {
                open: 0,
                close: 0,
                high: 0,
                low: 0,
                volume: 0,
            })),
        }
    }
}

impl std::fmt::Debug for Candlestick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let this = self.inner.read();

        f.write_fmt(format_args!(
            "MeticsCandlestick {{ open: {}, close: {}, high: {}, low {}, volume: {} }}",
            this.open,
            if this.volume > 1 { this.close / this.volume } else { this.close },
            this.high,
            this.low,
            this.volume,
        ))
    }
}

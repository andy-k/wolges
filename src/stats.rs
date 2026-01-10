// Copyright (C) 2020-2026 Andy Kurnia.

pub struct Stats {
    count: f64, // should be a non-negative int barring overflows
    mean: f64,
    m2: f64,
}

impl Default for Stats {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl Stats {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            count: 0.0,
            mean: 0.0,
            m2: 0.0,
        }
    }

    // https://en.wikipedia.org/wiki/Algorithms_for_calculating_variance#Welford's_online_algorithm
    #[inline(always)]
    pub fn update(&mut self, new_value: f64) {
        self.count += 1.0;
        let delta = new_value - self.mean;
        self.mean += delta / self.count;
        let delta2 = new_value - self.mean;
        self.m2 += delta * delta2;
    }

    // https://en.wikipedia.org/wiki/Algorithms_for_calculating_variance#Parallel_algorithm
    #[inline(always)]
    pub fn update_bulk(&mut self, other: &Stats) {
        let original_count = self.count;
        self.count += other.count;
        if self.count != 0.0 {
            // this branch is predictable
            let delta = other.mean - self.mean;
            let delta_mean = delta * (other.count / self.count);
            self.mean += delta_mean;
            self.m2 += other.m2 + delta * delta_mean * original_count;
        }
    }

    #[inline(always)]
    pub fn count(&self) -> f64 {
        self.count
    }

    #[inline(always)]
    pub fn mean(&self) -> f64 {
        self.mean
    }

    #[inline(always)]
    pub fn variance(&self) -> f64 {
        // this branch is largely predictable
        if self.count < 2.0 {
            0.0
        } else {
            self.m2 / (self.count - 1.0)
        }
    }

    #[inline(always)]
    pub fn standard_deviation(&self) -> f64 {
        self.variance().sqrt()
    }

    // https://www.mathsisfun.com/data/confidence-interval.html
    #[inline(always)]
    pub fn ci_max(&self, z: f64) -> f64 {
        self.mean + z * (self.variance() / self.count).sqrt()
    }
}

pub struct NormalDistribution {}

impl NormalDistribution {
    const START: f64 = -8.25;
    const STEP: f64 = 1.0f64 / 131072.0;

    // https://en.wikipedia.org/wiki/Normal_distribution#Standard_normal_distribution
    #[inline(always)]
    pub fn normal_density(x: f64) -> f64 {
        // FRAC_1_SQRT_2PI is nightly-only.
        (x * x * -0.5).exp()
            * (std::f64::consts::FRAC_1_SQRT_2 * std::f64::consts::FRAC_2_SQRT_PI * 0.5)
    }

    // approximate and asymmetrical.
    #[inline(always)]
    pub fn sum_normal_density(x_lo: f64, x_hi: f64) -> f64 {
        let mut ret = 0.0;
        let mut x = x_lo;
        while x <= x_hi {
            ret += Self::normal_density(x);
            x += Self::STEP;
        }
        ret * Self::STEP
    }

    // exclude unproductive range.
    // this is a single-use version.
    #[inline(always)]
    #[allow(unused)]
    pub fn cumulative_normal_density(x: f64) -> f64 {
        Self::sum_normal_density(Self::START, x.min(-Self::START))
    }

    #[inline(always)]
    pub fn reverse_ci(x: f64) -> f64 {
        let mut lo = 0.0;
        let mut hi = -Self::START;
        loop {
            let mid = (lo + hi) * 0.5;
            if (hi - lo) < Self::STEP {
                return mid;
            }
            match Self::sum_normal_density(-mid, mid).partial_cmp(&x) {
                Some(std::cmp::Ordering::Less) => lo = mid,
                Some(std::cmp::Ordering::Greater) => hi = mid,
                _ => return mid,
            }
        }
    }
}

pub struct CumulativeNormalDensity {
    cache: Vec<f64>, // 2162689 elements worst case.
    cache_cum: f64,
}

impl Default for CumulativeNormalDensity {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl CumulativeNormalDensity {
    pub fn new() -> Self {
        Self {
            cache: Vec::new(),
            cache_cum: 0.0,
        }
    }

    pub fn get(&mut self, x: f64) -> f64 {
        if x < NormalDistribution::START {
            return 0.0;
        }
        let i = ((x.min(-NormalDistribution::START) - NormalDistribution::START)
            / NormalDistribution::STEP)
            .floor() as usize;
        if i < self.cache.len() {
            return self.cache[i];
        }
        let mut v = self.cache.len() as f64 * NormalDistribution::STEP + NormalDistribution::START;
        loop {
            self.cache_cum += NormalDistribution::normal_density(v) * NormalDistribution::STEP;
            self.cache.push(self.cache_cum);
            if i < self.cache.len() {
                return self.cache_cum;
            }
            v += NormalDistribution::STEP;
        }
    }
}

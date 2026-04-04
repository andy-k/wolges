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
    // Abramowitz & Stegun 26.2.17 (5-term rational approximation).
    // Maximum error ~1.0e-7, sufficient for 4+ decimal places.
    // https://en.wikipedia.org/wiki/Normal_distribution#Numerical_approximations_for_the_normal_cumulative_distribution_function_and_normal_quantile_function
    #[inline(always)]
    pub fn cumulative_normal_density(x: f64) -> f64 {
        if x >= 0.0 {
            Self::cnd_positive(x)
        } else {
            1.0 - Self::cnd_positive(-x)
        }
    }

    #[inline(always)]
    fn cnd_positive(x: f64) -> f64 {
        const P: f64 = 0.2316419;
        const B1: f64 = 0.319381530;
        const B2: f64 = -0.356563782;
        const B3: f64 = 1.781477937;
        const B4: f64 = -1.821255978;
        const B5: f64 = 1.330274429;
        let t = 1.0 / (1.0 + P * x);
        let pdf = (x * x * -0.5).exp()
            * (std::f64::consts::FRAC_1_SQRT_2 * std::f64::consts::FRAC_2_SQRT_PI * 0.5);
        1.0 - pdf * t * (B1 + t * (B2 + t * (B3 + t * (B4 + t * B5))))
    }

    #[inline(always)]
    pub fn reverse_ci(x: f64) -> f64 {
        let mut lo = 0.0f64;
        let mut hi = 8.25f64;
        for _ in 0..64 {
            let mid = (lo + hi) * 0.5;
            let cdf_range =
                Self::cumulative_normal_density(mid) - Self::cumulative_normal_density(-mid);
            if cdf_range < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) * 0.5
    }
}

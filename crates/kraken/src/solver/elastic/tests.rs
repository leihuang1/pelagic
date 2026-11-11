use super::*;

impl SolidMesh {
    // Frozen pre-specialization step/transfer: keep the complex state and unit scaling.
    fn reference_step(
        &self,
        x: Complex64,
        y: [Complex64; 5],
        node: usize,
        euler: bool,
    ) -> [Complex64; 5] {
        let (b, rho) = self.coefficients[node.min(self.coefficients.len() - 1)];
        if self.real {
            let [b1, b2, b3, b4] = b.map(|b| b.re);
            let y = y.map(|y| y.re);
            let two_x = 2.0 * x.re;
            let four_h_x = self.h * (4.0 * x.re);
            let xb3 = x.re * b3 - rho;
            let fourth = if euler {
                xb3 * y[0] + (b2 * y[1] - b4 * (two_x * y[2]))
            } else {
                b2 * y[1] + (xb3 * y[0] - two_x * (b4 * y[2]))
            };
            let fifth = if euler {
                (rho * y[0] - four_h_x * y[2]) - b1 * y[1]
            } else {
                (rho * y[0] - b1 * y[1]) - four_h_x * y[2]
            };
            return [
                b1 * y[3] - b2 * y[4],
                -(rho * y[3] + xb3 * y[4]),
                2.0 * self.h * y[3] + b4 * y[4],
                fourth,
                fifth,
            ]
            .map(Complex64::from);
        }
        let [b1, b2, b3, b4] = b;
        let xb3 = x * b3 - rho;
        [
            b1 * y[3] - b2 * y[4],
            -rho * y[3] - xb3 * y[4],
            2.0 * self.h * y[3] + b4 * y[4],
            xb3 * y[0] + b2 * y[1] - 2.0 * x * b4 * y[2],
            rho * y[0] - b1 * y[1] - 4.0 * self.h * x * y[2],
        ]
    }

    fn reference_transfer(
        &self,
        x: Complex64,
        mut y: [Complex64; 5],
        top: bool,
        power: &mut i32,
    ) -> [Complex64; 5] {
        let sign = if top { 1.0 } else { -1.0 };
        let delta = self.reference_step(x, y, if top { 0 } else { self.intervals }, true);
        let mut z = std::array::from_fn(|i| y[i] + sign * 0.5 * delta[i]);
        let mut previous = y;
        for step in 0..self.intervals {
            previous = y;
            y = z;
            let node = if top {
                step + 1
            } else {
                self.intervals - step - 1
            };
            let delta = self.reference_step(x, y, node, false);
            z = std::array::from_fn(|i| previous[i] + sign * delta[i]);
            if step + 1 != self.intervals {
                let scale = if z[1].re.abs() < 1e-50 {
                    *power -= 50;
                    1e50
                } else if z[1].re.abs() > 1e50 {
                    *power += 50;
                    1e-50
                } else {
                    1.0
                };
                for i in 0..5 {
                    y[i] *= scale;
                    z[i] *= scale;
                }
            }
        }
        std::array::from_fn(|i| {
            if self.real {
                ((previous[i] + z[i]) + 2.0 * y[i]) * 0.25
            } else {
                (previous[i] + 2.0 * y[i] + z[i]) / 4.0
            }
        })
    }
}

fn check(mesh: &SolidMesh, x: Complex64, y: [Complex64; 5], top: bool) -> i32 {
    let mut power = 17;
    let mut expected_power = power;
    let expected = mesh.reference_transfer(x, y, top, &mut expected_power);
    let actual = mesh.transfer(x, y, top, &mut power);
    let bits = |values: [Complex64; 5]| values.map(|v| (v.re.to_bits(), v.im.to_bits()));
    assert_eq!(
        bits(actual),
        bits(expected),
        "intervals={}, x={x}, y={y:?}, top={top}",
        mesh.intervals
    );
    assert_eq!(power, expected_power);
    power - 17
}

fn random(seed: &mut u32) -> f64 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    f64::from(*seed) / f64::from(u32::MAX) - 0.5
}

#[test]
fn real_transfer_matches_complex_state_bit_for_bit() {
    let mut seed = 737;
    for intervals in [1, 2, 3, 16, 201] {
        for sampled in [false, true] {
            let mesh = SolidMesh {
                intervals,
                h: 0.003,
                coefficients: (0..if sampled { intervals + 1 } else { 1 })
                    .map(|_| {
                        (
                            std::array::from_fn(|_| Complex64::from(random(&mut seed))),
                            random(&mut seed),
                        )
                    })
                    .collect(),
                minimum_speed: 1400.0,
                real: true,
            };
            for scale in [1e-200, 1e-51, 1e-50, 1.0, 1e50, 1e51, 1e200] {
                for _ in 0..64 {
                    let y = std::array::from_fn(|_| {
                        Complex64::new(
                            scale * random(&mut seed),
                            0.0_f64.copysign(random(&mut seed)),
                        )
                    });
                    for x in [0.0, -0.0, random(&mut seed)] {
                        for top in [false, true] {
                            check(&mesh, x.into(), y, top);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn real_transfer_rescale_thresholds_and_signed_zeros() {
    let mut mesh = SolidMesh {
        intervals: 2,
        h: 0.0,
        coefficients: vec![([Complex64::from(0.0); 4], 0.0)],
        minimum_speed: 1400.0,
        real: true,
    };
    for (magnitude, expected_power) in [
        (0.0, -50),
        (1e-50_f64.next_down(), -50),
        (1e-50, 0),
        (1e-50_f64.next_up(), 0),
        (1e50_f64.next_down(), 0),
        (1e50, 0),
        (1e50_f64.next_up(), 50),
        (1e-200, -50),
        (1e200, 50),
    ] {
        for sign in [-1.0, 1.0] {
            for top in [false, true] {
                let y = [Complex64::new(sign * magnitude, sign * 0.0); 5];
                assert_eq!(check(&mesh, 0.0.into(), y, top), expected_power);
                mesh.intervals = 1;
                assert_eq!(check(&mesh, (-0.0).into(), y, top), 0);
                mesh.intervals = 2;
            }
        }
    }
    // Every combination of signed real/imaginary zeros, including repeated scaling.
    mesh.intervals = 5;
    for mask in 0..1024 {
        let y = std::array::from_fn(|i| {
            Complex64::new(
                if mask & (1 << i) == 0 { 0.0 } else { -0.0 },
                if mask & (1 << (i + 5)) == 0 {
                    0.0
                } else {
                    -0.0
                },
            )
        });
        for top in [false, true] {
            assert_eq!(check(&mesh, (-0.0).into(), y, top), -200);
        }
    }
}

#[test]
fn complex_transfer_keeps_its_arithmetic() {
    let mut seed = 91;
    for intervals in [1, 2, 31] {
        let mesh = SolidMesh {
            intervals,
            h: 0.03,
            coefficients: (0..=intervals)
                .map(|_| {
                    (
                        std::array::from_fn(|_| {
                            Complex64::new(random(&mut seed), random(&mut seed))
                        }),
                        random(&mut seed),
                    )
                })
                .collect(),
            minimum_speed: 1400.0,
            real: false,
        };
        for scale in [1e-100, 1.0, 1e100] {
            for _ in 0..64 {
                let x = Complex64::new(random(&mut seed), random(&mut seed));
                let y = std::array::from_fn(|_| {
                    scale * Complex64::new(random(&mut seed), random(&mut seed))
                });
                for top in [false, true] {
                    check(&mesh, x, y, top);
                }
            }
        }
    }
}

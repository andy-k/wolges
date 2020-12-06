#[derive(Clone, Copy)]
struct RowCol(i8, i8);

struct Premium {
  lm: i8,
  wm: i8,
}

enum PremiumEnum {
  A = Premium {wm:3,lm:1},
  B = Premium {wm:3,lm:1},
}

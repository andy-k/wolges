trait TA<'a> {
  fn st(&self) -> &'a str;
}

struct SA<'a> {
  st: &'a str,
}

impl<'a> TA<'a> for SA<'a> {
  fn st(&self) -> &'a str { self.st }
}

fn wat(ta: &dyn TA) {
  println!("{}", ta.st());
}

trait TB<'a> {
  fn s2(&self) -> &'a str;
}

struct SB<'a> {
  ta: &'a (dyn TA<'a>+Sync),
}

impl<'a> TB<'a> for SB<'a> {
  fn s2(&self) -> &'a str { self.ta.st() }
}

fn wat2(ta: &dyn TB) {
  println!("{}", ta.s2());
}

  static sa : &SA = &SA { st: "hello" };
  static sb : &SB = &SB { ta: sa };

fn main() {
  //let sa = &SA { st: "hello" };
  wat(sa);
  wat(sa);
  //let sb = &SB { ta: sa };
  wat2(sb);
  wat2(sb);
  wat(sb.ta);
  wat(sa);
}

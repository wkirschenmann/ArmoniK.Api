//! Many distinct callsites, so that the cost of rebuilding the interest cache can be measured
//! against a process that has registered as many as a real one.

macro_rules! one {
    () => {
        tracing::debug!(target: "armonik_transport::bulk", "bulk")
    };
}

macro_rules! times4 {
    ($m:ident) => {
        $m!();
        $m!();
        $m!();
        $m!();
    };
}

macro_rules! times16 {
    ($m:ident) => {
        times4!($m);
        times4!($m);
        times4!($m);
        times4!($m);
    };
}

#[inline(never)]
fn sixteen() {
    times16!(one);
}

#[inline(never)]
fn two_fifty_six() {
    sixteen_each();
}

macro_rules! sixteen_calls {
    () => {
        times16!(one);
    };
}

#[inline(never)]
fn sixteen_each() {
    // 16 functions of 16 callsites each, spelled out so that every one is its own callsite.
    f0();
    f1();
    f2();
    f3();
    f4();
    f5();
    f6();
    f7();
    f8();
    f9();
    f10();
    f11();
    f12();
    f13();
    f14();
    f15();
}

macro_rules! define_fns {
    ($($name:ident),*) => {
        $(
            #[inline(never)]
            fn $name() {
                sixteen_calls!();
            }
        )*
    };
}

define_fns!(f0, f1, f2, f3, f4, f5, f6, f7, f8, f9, f10, f11, f12, f13, f14, f15);

/// Reaches 256 distinct callsites once each, registering them.
pub fn touch_256() {
    two_fifty_six();
}

#[inline(never)]
fn block_a() {
    touch_256();
}

macro_rules! block_fns {
    ($($name:ident),*) => {
        $(
            #[inline(never)]
            fn $name() {
                // A second copy of the 256 is another 256 callsites only if the code is
                // duplicated, so each block has its own 16x16 expansion.
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
                sixteen_calls!();
            }
        )*
    };
}

block_fns!(b0, b1, b2, b3, b4, b5, b6, b7, b8, b9, b10, b11, b12, b13, b14, b15);

/// Reaches 4096 more distinct callsites, in addition to the 256.
pub fn touch_4096() {
    block_a();
    b0();
    b1();
    b2();
    b3();
    b4();
    b5();
    b6();
    b7();
    b8();
    b9();
    b10();
    b11();
    b12();
    b13();
    b14();
    b15();
    let _ = sixteen;
}

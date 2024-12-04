use std::io;

fn main() {
    println!(
        r#"---- DHCP Client ----
1. Request one ip address from random server node
"#
    );

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    match input.trim() {
        "1" => unimplemented!(),
        _ => panic!(),
    }
}

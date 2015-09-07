///  Program for testing and playing with the parser
///

extern crate uosql;
use uosql::parse;



fn main() {


    let mut p = parse::Parser::create("insert into random_table () values (asd,dsad, asd)");

    println!("{:?}",p.parse());


}

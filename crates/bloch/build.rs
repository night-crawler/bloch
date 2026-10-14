fn main() {
    lalrpop::process_src().expect("failed to generate the type-header parser");
}

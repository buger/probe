fn main() {
    println!("Hello from main!");
    helper_function();
    another_function();
}

fn helper_function() {
    println!("This is a helper function");
    inner_function();
}

fn inner_function() {
    println!("This is an inner function");
}

fn another_function() {
    println!("Another function that calls helper");
    helper_function();
}

pub struct SimpleStruct {
    pub value: i32,
}

impl SimpleStruct {
    pub fn new(value: i32) -> Self {
        Self { value }
    }

    pub fn get_value(&self) -> i32 {
        self.value
    }

    pub fn set_value(&mut self, value: i32) {
        self.value = value;
    }
}

pub fn utility_function() -> SimpleStruct {
    SimpleStruct::new(42)
}

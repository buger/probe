package main

import "fmt"

// Person represents a basic person
type Person struct {
	Name string
	Age  int
}

// SayHello method for Person
func (p Person) SayHello() string {
	return fmt.Sprintf("Hello, I'm %s", p.Name)
}

// Address represents a person's address
type Address struct {
	Street string
	City   string
	State  string
}

// GetFullAddress method for Address
func (a Address) GetFullAddress() string {
	return fmt.Sprintf("%s, %s, %s", a.Street, a.City, a.State)
}

// Employee represents an employee with nested Person and Address
type Employee struct {
	Person
	Address
	Salary int
}

// DisplayInfo method for Employee
func (e Employee) DisplayInfo() string {
	return fmt.Sprintf("Employee: %s, Address: %s", e.SayHello(), e.GetFullAddress())
}

// GetSalaryDetails method for Employee
func (e Employee) GetSalaryDetails() string {
	return fmt.Sprintf("Salary: $%d", e.Salary)
}

// CalculateBonus method for Employee
func (e Employee) CalculateBonus() int {
	return e.Salary / 10
}

// NestedFunction is a standalone function
func NestedFunction() string {
	return "This is a nested function"
}

func main() {
	emp := Employee{
		Person: Person{Name: "John", Age: 30},
		Address: Address{Street: "123 Main St", City: "Anytown", State: "CA"},
		Salary: 50000,
	}
	
	fmt.Println(emp.DisplayInfo())
	fmt.Println(emp.GetSalaryDetails())
	fmt.Printf("Bonus: $%d\n", emp.CalculateBonus())
	fmt.Println(NestedFunction())
}
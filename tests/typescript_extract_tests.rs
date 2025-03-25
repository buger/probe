use std::fs;

// Import necessary functions from the extract module
use probe::extract::process_file_for_extraction;

fn execute_test(content: &str, expected_outputs: Vec<(usize, usize, usize)>) {
    // Create a temporary file with JavaScript code for testing
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.ts");

    // Write the content to the temporary file
    fs::write(&file_path, content).unwrap();

    for (line_number, expected_start, expected_end) in expected_outputs {
        // Call process_file_for_extraction for the current line number
        let result =
            process_file_for_extraction(&file_path, Some(line_number), None, None, false, 0, None)
                .unwrap();

        // Compare outputs against the expected output structure
        assert_eq!(result.file, file_path.to_string_lossy().to_string());
        assert!(
            result.lines.0 == expected_start && result.lines.1 == expected_end,
            "Line: {} | Expected: ({}, {}) | Actual: ({}, {})\nCode:{}",
            line_number,
            expected_start,
            expected_end,
            result.lines.0,
            result.lines.1,
            result.code
        );
    }
}

#[test]
fn test_typescript_extraction_react() {
    /* Code provided by Facebook in their React Typescript Tutorial: https://react.dev/learn/typescript */
    let content = r#"
import { createContext, useContext, useState, useMemo } from 'react';

// This is a simpler example, but you can imagine a more complex object here
type ComplexObject = {
	kind: string
};

// The context is created with `| null` in the type, to accurately reflect the default value.
const Context = createContext<ComplexObject | null>(null);

// The `| null` will be removed via the check in the Hook.
const useGetComplexObject = () => {
	const object = useContext(Context);
	if (!object) { throw new Error("useGetComplexObject must be used within a Provider") }
	return object;
}

export default function MyApp() {
	const object = useMemo(() => ({ kind: "complex" }), []);

	return (
		<Context.Provider value={object}>
			<MyComponent />
		</Context.Provider>
	)
}

function MyComponent() {
	const object = useGetComplexObject();

	return (
		<div>
			<p>Current object: {object.kind}</p>
		</div>
	)
}
"#;

    let expected_outputs = vec![
        (1, 1, 1), // initial blank line
        (2, 2, 2), // import statement
        // BUG? (3, 2, 37), // blank line -> entire module
        (4, 4, 7), // comment + type ComplexObject
        (5, 5, 7), // type ComplexObject
        (6, 5, 7), // type ComplexObject
        (7, 5, 7), // type ComplexObject
        // BUG? (8, 2, 37), // blank line -> entire module
        (9, 9, 17), // comment line -> matches following code up to end of next acceptable element.
        (10, 10, 10), // single code line
        // BUG? (11, 2, 37), // blank line -> entire module
        (12, 12, 17), // comment -> matches following function declaration
        // BUG? (13, 13, 17), // useGetComplexObject function
        (14, 13, 17), // useGetComplexObject function
        (15, 13, 17), // useGetComplexObject function
        (16, 13, 17), // useGetComplexObject function
        (17, 13, 17), // useGetComplexObject function
        // BUG? (18, 2, 37), // blank line -> entire module
        (19, 19, 27), // MyApp function
        (20, 19, 27), // MyApp function
        (21, 19, 27), // MyApp function
        (22, 19, 27), // MyApp function
        (23, 19, 27), // !! BUG - should be 23-25 <Context.Provider> JSX element.  Works in JS but not TS...
        (24, 19, 27), // !! BUG - should be 24-24 <MyComponent/> JSX element.
        (25, 19, 27), // !! BUG - should be 23-25 <Context.Provider> JSX element.  Works in JS but not TS...
        (26, 19, 27), // MyApp function
        (27, 19, 27), // MyApp function
        // BUG? (28, 2, 37), // blank line -> entire module
        // BUG? (29, 29, 37), // MyComponent function
        (30, 29, 37), // MyComponent function
        (31, 29, 37), // MyComponent function
        (32, 29, 37), // MyComponent function
        (33, 29, 37), // !! BUG - should be 33-35 <div> JSX element
        (34, 29, 37), // !! BUG - should be 34-34 <p> JSX element
        (35, 29, 37), // !! BUG - should be 33-35 <div> JSX element
        (36, 29, 37), // MyComponent function
        (37, 29, 37), // MyComponent function
    ];

    execute_test(content, expected_outputs);
}

#[test]
fn test_typescript_extraction_types() {
    /* Various complex Typescript types taken from the Typescript Handbook:
    https://www.typescriptlang.org/docs/handbook/ */
    let content = r#"
declare function create<T extends HTMLElement = HTMLDivElement, U extends HTMLElement[] = T[]>(
  element?: T,
  children?: U
): Container<T, U>;

function printTextOrNumberOrBool(
  textOrNumberOrBool:
    | string
    | number
    | boolean
) {
  console.log(textOrNumberOrBool);
}

// It is also good to test comments...
// including multi-line comments.
type Shape =
  | Circle
  | Square
  | Triangle

interface PaintOptions {
  shape: Shape;
  xPos?: number;
  yPos?: number;
}

declare namespace GreetingLib {
  interface LogOptions {
    verbose?: boolean;
  }
  interface AlertOptions {
    modal: boolean;
    title?: string;
    color?: string;
  }
}
"#;

    let expected_outputs = vec![
        (1, 1, 1), // initial blank line
        (2, 2, 2), // !! BUG - should match lines 2-5
        (3, 3, 3), // !! BUG - should match lines 2-5
        (4, 4, 4), // !! BUG - should match lines 2-5
        (5, 5, 5), // !! BUG - should match lines 2-5
        // BUG (6, 2, 38), // !! BUG - blank line - should match entire module
        // BUG (7, 7, 14), // printTextOrNumberOrBool function
        (8, 7, 14),  // printTextOrNumberOrBool function
        (9, 7, 14),  // printTextOrNumberOrBool function
        (10, 7, 14), // printTextOrNumberOrBool function
        (11, 7, 14), // printTextOrNumberOrBool function
        (12, 7, 14), // printTextOrNumberOrBool function
        (13, 7, 14), // printTextOrNumberOrBool function
        (14, 7, 14), // printTextOrNumberOrBool function
        // BUG? (15, 2, 38), // blank line -> entire module
        (16, 16, 21), // Comment + Shape type alias
        (17, 17, 21), // Comment + Shape type alias
        (18, 18, 21), // Shape type alias
        (19, 18, 21), // Shape type alias
        (20, 18, 21), // Shape type alias
        (21, 18, 21), // Shape type alias
        // BUG? (22, 2, 38), // blank line -> entire module
        (23, 23, 27), // PaintOptions interface
        (24, 23, 27), // PaintOptions interface
        (25, 23, 27), // PaintOptions interface
        (26, 23, 27), // PaintOptions interface
        (27, 23, 27), // PaintOptions interface
        // BUG? (28, 2, 38), // blank line -> entire module
        (29, 29, 29), // !! BUG - should be namespace GreetingLib 29-38
        (30, 30, 32), // interface LogOptions
        (31, 30, 32), // interface LogOptions
        (32, 30, 32), // interface LogOptions
        (33, 33, 37), // interface AlertOptions
        (34, 33, 37), // interface AlertOptions
        (35, 33, 37), // interface AlertOptions
        (36, 33, 37), // interface AlertOptions
        (37, 33, 37), // interface AlertOptions
        (38, 38, 38), // !! BUG - should be namespace GreetingLib 29-38
    ];

    execute_test(content, expected_outputs);
}

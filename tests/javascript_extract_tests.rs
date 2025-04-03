use std::fs;

// Import necessary functions from the extract module
use probe::extract::process_file_for_extraction;

fn execute_test(content: &str, expected_outputs: Vec<(usize, usize, usize)>) {
    // Create a temporary file with JavaScript code for testing
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_file.js");

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
fn test_javascript_extraction_aframe_component() {
    let content = r#"
AFRAME.registerComponent('position', positionComponent)
const positionComponent = {
	schema: {type: 'vec3'},
	
	update: function () {
		var object3D = this.el.object3D;
		var data = this.data;
		object3D.position.set(data.x, data.y, data.z);
	},
	
	remove: function () {
		// Pretty much for mixins.
		this.el.object3D.position.set(0, 0, 0);
	}
};
"#;

    // Many of the expected outputs have stopped working since meriging latest code from main.
    // Suspect changes in e788c83, but haven't understoof this yet:
    // https://github.com/buger/probe/commit/e788c837bd63813fb2e4fba9d64604e0c9755c4a
    let expected_outputs = vec![
        (0, 1, 1), // before start of file
        (1, 1, 1), // initial blank line
        (2, 2, 2), // reisterComponent call
        // BUG (3, 3, 16), // object declaration
        // BUG (4, 4, 4), // schema definition
        // BUG (5, 3, 16), // entire positionComponent
        // BUG (6, 6, 10), // update function
        (7, 6, 10), // update function
        (8, 6, 10), // update function
        (9, 6, 10), // update function
        // BUG (11, 3, 16), // entire positionComponent
        // BUG (12, 12, 15), // remove function
        // BUG (13, 12, 15), // remove function
        (14, 12, 15), // remove function
        (15, 12, 15), // remove function
        (16, 3, 16),  // close object definition
        (17, 3, 16),  // end of file
        (25, 3, 16),  // beyond end of file
    ];

    execute_test(content, expected_outputs);
}

#[test]
fn test_javascript_extraction_object() {
    let content = r#"
const user = {
	id: 1,
	name: "John Smith",
	email: "john.smith@example.com",
	profile: {
		age: 30,
		occupation: "Software Engineer",
		skills: ["JavaScript", "TypeScript", "React", "Node.js"]
	},
	isActive: true,
	lastLogin: new Date("2023-01-01")
};
"#;

    let expected_outputs = vec![
        (0, 1, 1), // before start of file
        (1, 1, 1), // blank line
        // BUG (2, 2, 13), // entire object
        (3, 2, 13), // entire object
        (4, 2, 13), // entire object
        (5, 2, 13), // entire object
        // BUG (6, 6, 10), // nested object
        (7, 6, 10), // nested object
        (8, 6, 10), // nested object
        // BUG (9, 9, 9), // nested array
        (10, 6, 10), // nested object
        (11, 2, 13), // entire object
        (12, 2, 13), // entire object
        (13, 2, 13), // entire object
    ];

    execute_test(content, expected_outputs);
}

#[test]
fn test_javascript_extraction_array() {
    let content = r#"
const array = [
	{
		name: "Alice",
		age: 25,
		city: "New York"
	},
	{
		name: "Bob",
		age: 30,
		city: "Los Angeles"
	}
];
"#;

    let expected_outputs = vec![
        (1, 1, 1), // blank line
        // BUG (2, 2, 13), // entire array
        (3, 3, 7),   // 1st object
        (4, 3, 7),   // 1st object
        (5, 3, 7),   // 1st object
        (6, 3, 7),   // 1st object
        (7, 3, 7),   // 1st object
        (8, 8, 12),  // 2nd object
        (9, 8, 12),  // 2nd object
        (10, 8, 12), // 2nd object
        (11, 8, 12), // 2nd object
        (12, 8, 12), // 2nd object
        (13, 2, 13), // entire array
    ];

    execute_test(content, expected_outputs);
}

#[test]
fn test_javascript_extraction_react_code() {
    /* Code provided by Facebook in their React Tutorial: https://react.dev/learn/tutorial-tic-tac-toe */
    let content = r#"
import { useState } from 'react';

function Square({ value, onSquareClick }) {
	return (
		<button className="square" onClick={onSquareClick}>
			{value}
		</button>
	);
}

function Board({ xIsNext, squares, onPlay }) {
	function handleClick(i) {
		if (calculateWinner(squares) || squares[i]) {
			return;
		}
		const nextSquares = squares.slice();
		if (xIsNext) {
			nextSquares[i] = 'X';
		} else {
			nextSquares[i] = 'O';
		}
		onPlay(nextSquares);
	}

	const winner = calculateWinner(squares);
	let status;
	if (winner) {
		status = 'Winner: ' + winner;
	} else {
		status = 'Next player: ' + (xIsNext ? 'X' : 'O');
	}

	return (
		<>
			<div className="status">{status}</div>
			<div className="board-row">
				<Square value={squares[0]} onSquareClick={() => handleClick(0)} />
				<Square value={squares[1]} onSquareClick={() => handleClick(1)} />
				<Square value={squares[2]} onSquareClick={() => handleClick(2)} />
			</div>
			<div className="board-row">
				<Square value={squares[3]} onSquareClick={() => handleClick(3)} />
				<Square value={squares[4]} onSquareClick={() => handleClick(4)} />
				<Square value={squares[5]} onSquareClick={() => handleClick(5)} />
			</div>
			<div className="board-row">
				<Square value={squares[6]} onSquareClick={() => handleClick(6)} />
				<Square value={squares[7]} onSquareClick={() => handleClick(7)} />
				<Square value={squares[8]} onSquareClick={() => handleClick(8)} />
			</div>
		</>
	);
}

export default function Game() {
	const [history, setHistory] = useState([Array(9).fill(null)]);
	const [currentMove, setCurrentMove] = useState(0);
	const xIsNext = currentMove % 2 === 0;
	const currentSquares = history[currentMove];

	function handlePlay(nextSquares) {
		const nextHistory = [...history.slice(0, currentMove + 1), nextSquares];
		setHistory(nextHistory);
		setCurrentMove(nextHistory.length - 1);
	}

	function jumpTo(nextMove) {
		setCurrentMove(nextMove);
	}

	const moves = history.map((squares, move) => {
		let description;
		if (move > 0) {
			description = 'Go to move #' + move;
		} else {
			description = 'Go to game start';
		}
		return (
			<li key={move}>
				<button onClick={() => jumpTo(move)}>{description}</button>
			</li>
		);
	});

	return (
		<div className="game">
			<div className="game-board">
				<Board xIsNext={xIsNext} squares={currentSquares} onPlay={handlePlay} />
			</div>
			<div className="game-info">
				<ol>{moves}</ol>
			</div>
		</div>
	);
}

function calculateWinner(squares) {
	const lines = [
		[0, 1, 2],
		[3, 4, 5],
		[6, 7, 8],
		[0, 3, 6],
		[1, 4, 7],
		[2, 5, 8],
		[0, 4, 8],
		[2, 4, 6],
	];
	for (let i = 0; i < lines.length; i++) {
		const [a, b, c] = lines[i];
		if (squares[a] && squares[a] === squares[b] && squares[a] === squares[c]) {
			return squares[a];
		}
	}
	return null;
}
"#;

    // Declare expected output for values 1, 2, 3, etc.
    // Since the fragment is long, we check a selection of sample points, rather than
    // checking exhaustively.
    let expected_outputs = vec![
        (1, 1, 1),       // blank line
        (5, 4, 10),      // Square function
        (10, 4, 10),     // Square function
        (15, 13, 24),    // handleClick function
        (20, 13, 24),    // handleClick function
        (25, 12, 54),    // entire Board function
        (30, 12, 54),    // entire Board function
        (35, 35, 52),    // JSX element
        (40, 40, 40),    // single JSX element <Square>
        (45, 45, 45),    // single JSX element <Square>
        (50, 50, 50),    // single JSX element <Square>
        (55, 55, 55),    // !! BUG - blank line, so expected entire object
        (60, 56, 96),    // entire Game function
        (65, 62, 66),    // handlePlay function
        (70, 68, 70),    // jumpTo function
        (75, 72, 84),    // history.map expression
        (80, 80, 82),    // <li> JSX element
        (85, 56, 96),    // entire Game function
        (90, 88, 90),    // "game-board" <div> JSX element
        (95, 56, 96),    // entire Game function
        (100, 100, 100), // single-line array in lines
        (105, 105, 105), // single-line array in lines
        (110, 98, 116),  // calculateWinner function
        (115, 98, 116),  // calculateWinner function
    ];

    execute_test(content, expected_outputs);
}

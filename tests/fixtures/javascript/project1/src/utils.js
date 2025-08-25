// Utility functions module

/**
 * Add two numbers together
 * This function should show incoming calls from calculate and other functions
 * @param {number} x First number
 * @param {number} y Second number
 * @returns {number} Sum of x and y
 */
function add(x, y) {
    return x + y;
}

/**
 * Multiply two numbers
 * This function should show incoming calls from calculate and other functions
 * @param {number} x First number
 * @param {number} y Second number
 * @returns {number} Product of x and y
 */
function multiply(x, y) {
    return x * y;
}

/**
 * Subtract two numbers
 * This function should show incoming calls from calculate
 * @param {number} x First number
 * @param {number} y Second number
 * @returns {number} Difference of x and y
 */
function subtract(x, y) {
    return x - y;
}

/**
 * Divide two numbers with safety check
 * This function might not have incoming calls in our test
 * @param {number} x Dividend
 * @param {number} y Divisor
 * @returns {number} Quotient
 */
function divide(x, y) {
    if (y === 0) {
        throw new Error("Division by zero");
    }
    return x / y;
}

/**
 * Utility helper that demonstrates chained function calls
 * @param {number} a First input
 * @param {number} b Second input
 * @returns {number} Computed result
 */
function utilityHelper(a, b) {
    const temp = add(a, b);      // Outgoing call to add
    return multiply(temp, 3);    // Outgoing call to multiply
}

/**
 * Math utilities object for additional testing
 */
const MathUtils = {
    /**
     * Power function that uses multiply internally
     * @param {number} base Base number
     * @param {number} exponent Exponent (must be positive integer)
     * @returns {number} Result of base^exponent
     */
    power(base, exponent) {
        if (exponent === 0) return 1;
        if (exponent === 1) return base;
        
        let result = base;
        for (let i = 1; i < exponent; i++) {
            result = multiply(result, base); // Outgoing call to multiply
        }
        return result;
    },
    
    /**
     * Square function
     * @param {number} x Input number
     * @returns {number} Square of x
     */
    square(x) {
        return multiply(x, x); // Outgoing call to multiply
    }
};

/**
 * Array processing utilities
 */
const ArrayUtils = {
    /**
     * Sum all elements in an array
     * @param {number[]} arr Input array
     * @returns {number} Sum of all elements
     */
    sum(arr) {
        return arr.reduce((acc, val) => add(acc, val), 0); // Multiple calls to add
    },
    
    /**
     * Product of all elements in an array
     * @param {number[]} arr Input array
     * @returns {number} Product of all elements
     */
    product(arr) {
        return arr.reduce((acc, val) => multiply(acc, val), 1); // Multiple calls to multiply
    }
};

// Export all functions and utilities
module.exports = {
    add,
    multiply,
    subtract,
    divide,
    utilityHelper,
    MathUtils,
    ArrayUtils
};
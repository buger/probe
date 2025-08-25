"use strict";
// Utility functions module
Object.defineProperty(exports, "__esModule", { value: true });
exports.MathUtils = void 0;
exports.add = add;
exports.multiply = multiply;
exports.subtract = subtract;
exports.divide = divide;
exports.utilityHelper = utilityHelper;
/**
 * Add two numbers together
 * This function should show incoming calls from calculate and other functions
 * @param x First number
 * @param y Second number
 * @returns Sum of x and y
 */
function add(x, y) {
    return x + y;
}
/**
 * Multiply two numbers
 * This function should show incoming calls from calculate and other functions
 * @param x First number
 * @param y Second number
 * @returns Product of x and y
 */
function multiply(x, y) {
    return x * y;
}
/**
 * Subtract two numbers
 * This function should show incoming calls from calculate
 * @param x First number
 * @param y Second number
 * @returns Difference of x and y
 */
function subtract(x, y) {
    return x - y;
}
/**
 * Divide two numbers with safety check
 * This function might not have incoming calls in our test
 * @param x Dividend
 * @param y Divisor
 * @returns Quotient
 */
function divide(x, y) {
    if (y === 0) {
        throw new Error("Division by zero");
    }
    return x / y;
}
/**
 * Utility helper that demonstrates chained function calls
 * @param a First input
 * @param b Second input
 * @returns Computed result
 */
function utilityHelper(a, b) {
    const temp = add(a, b); // Outgoing call to add
    return multiply(temp, 3); // Outgoing call to multiply
}
/**
 * Math utilities namespace for additional testing
 */
var MathUtils;
(function (MathUtils) {
    /**
     * Power function that uses multiply internally
     * @param base Base number
     * @param exponent Exponent (must be positive integer)
     * @returns Result of base^exponent
     */
    function power(base, exponent) {
        if (exponent === 0)
            return 1;
        if (exponent === 1)
            return base;
        let result = base;
        for (let i = 1; i < exponent; i++) {
            result = multiply(result, base); // Outgoing call to multiply
        }
        return result;
    }
    MathUtils.power = power;
    /**
     * Square function
     * @param x Input number
     * @returns Square of x
     */
    function square(x) {
        return multiply(x, x); // Outgoing call to multiply
    }
    MathUtils.square = square;
})(MathUtils || (exports.MathUtils = MathUtils = {}));
//# sourceMappingURL=utils.js.map
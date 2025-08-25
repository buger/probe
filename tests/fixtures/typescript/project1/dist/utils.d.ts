/**
 * Add two numbers together
 * This function should show incoming calls from calculate and other functions
 * @param x First number
 * @param y Second number
 * @returns Sum of x and y
 */
export declare function add(x: number, y: number): number;
/**
 * Multiply two numbers
 * This function should show incoming calls from calculate and other functions
 * @param x First number
 * @param y Second number
 * @returns Product of x and y
 */
export declare function multiply(x: number, y: number): number;
/**
 * Subtract two numbers
 * This function should show incoming calls from calculate
 * @param x First number
 * @param y Second number
 * @returns Difference of x and y
 */
export declare function subtract(x: number, y: number): number;
/**
 * Divide two numbers with safety check
 * This function might not have incoming calls in our test
 * @param x Dividend
 * @param y Divisor
 * @returns Quotient
 */
export declare function divide(x: number, y: number): number;
/**
 * Utility helper that demonstrates chained function calls
 * @param a First input
 * @param b Second input
 * @returns Computed result
 */
export declare function utilityHelper(a: number, b: number): number;
/**
 * Math utilities namespace for additional testing
 */
export declare namespace MathUtils {
    /**
     * Power function that uses multiply internally
     * @param base Base number
     * @param exponent Exponent (must be positive integer)
     * @returns Result of base^exponent
     */
    function power(base: number, exponent: number): number;
    /**
     * Square function
     * @param x Input number
     * @returns Square of x
     */
    function square(x: number): number;
}
//# sourceMappingURL=utils.d.ts.map
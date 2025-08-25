// Utility functions module

/**
 * Add two numbers together
 * This function should show incoming calls from calculate and other functions
 * @param x First number
 * @param y Second number
 * @returns Sum of x and y
 */
export function add(x: number, y: number): number {
    return x + y;
}

/**
 * Multiply two numbers
 * This function should show incoming calls from calculate and other functions
 * @param x First number
 * @param y Second number
 * @returns Product of x and y
 */
export function multiply(x: number, y: number): number {
    return x * y;
}

/**
 * Subtract two numbers
 * This function should show incoming calls from calculate
 * @param x First number
 * @param y Second number
 * @returns Difference of x and y
 */
export function subtract(x: number, y: number): number {
    return x - y;
}

/**
 * Divide two numbers with safety check
 * This function might not have incoming calls in our test
 * @param x Dividend
 * @param y Divisor
 * @returns Quotient
 */
export function divide(x: number, y: number): number {
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
export function utilityHelper(a: number, b: number): number {
    const temp = add(a, b);      // Outgoing call to add
    return multiply(temp, 3);    // Outgoing call to multiply
}

/**
 * Math utilities namespace for additional testing
 */
export namespace MathUtils {
    /**
     * Power function that uses multiply internally
     * @param base Base number
     * @param exponent Exponent (must be positive integer)
     * @returns Result of base^exponent
     */
    export function power(base: number, exponent: number): number {
        if (exponent === 0) return 1;
        if (exponent === 1) return base;
        
        let result = base;
        for (let i = 1; i < exponent; i++) {
            result = multiply(result, base); // Outgoing call to multiply
        }
        return result;
    }
    
    /**
     * Square function
     * @param x Input number
     * @returns Square of x
     */
    export function square(x: number): number {
        return multiply(x, x); // Outgoing call to multiply
    }
}
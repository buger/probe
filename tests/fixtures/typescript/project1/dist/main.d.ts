/**
 * Main entry point of the application
 */
declare function main(): void;
/**
 * Processes an array of numbers using calculate function
 * This creates another incoming call to calculate
 * @param numbers Array of numbers to process
 * @returns Processed array
 */
declare function processNumbers(numbers: number[]): number[];
/**
 * Calculator class for testing method call hierarchy
 */
declare class Calculator {
    private multiplier;
    constructor(multiplier: number);
    /**
     * Instance method that calls calculate function
     * @param value Input value
     * @returns Processed value
     */
    processValue(value: number): number;
    /**
     * Static method for additional testing
     * @param x Input value
     * @returns Processed value
     */
    static staticProcess(x: number): number;
}
export { main, processNumbers, Calculator };
//# sourceMappingURL=main.d.ts.map
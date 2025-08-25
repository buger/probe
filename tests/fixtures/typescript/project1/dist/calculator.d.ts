/**
 * Calculate performs a complex calculation using utility functions
 * This function should show both incoming calls (from main, processNumbers, Calculator)
 * and outgoing calls (to add, multiply, subtract)
 * @param a First operand
 * @param b Second operand
 * @returns Calculated result
 */
export declare function calculate(a: number, b: number): number;
/**
 * Advanced calculation that also uses the calculate function
 * This creates another pathway in the call hierarchy
 * @param values Array of values to process
 * @returns Advanced result
 */
export declare function advancedCalculation(values: number[]): number;
/**
 * Business logic interface
 */
export interface IBusinessLogic {
    processValue(value: number): number;
}
/**
 * Complex business logic class
 */
export declare class BusinessLogic implements IBusinessLogic {
    private readonly multiplier;
    constructor(multiplier: number);
    /**
     * Process a value using internal logic
     * @param value Input value
     * @returns Processed result
     */
    processValue(value: number): number;
    /**
     * Complex processing that chains multiple calls
     * @param data Array of input data
     * @returns Processed results
     */
    processArray(data: number[]): number[];
}
//# sourceMappingURL=calculator.d.ts.map
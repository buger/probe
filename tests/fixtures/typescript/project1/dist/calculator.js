"use strict";
// Calculator module with core business logic
Object.defineProperty(exports, "__esModule", { value: true });
exports.BusinessLogic = void 0;
exports.calculate = calculate;
exports.advancedCalculation = advancedCalculation;
const utils_1 = require("./utils");
/**
 * Calculate performs a complex calculation using utility functions
 * This function should show both incoming calls (from main, processNumbers, Calculator)
 * and outgoing calls (to add, multiply, subtract)
 * @param a First operand
 * @param b Second operand
 * @returns Calculated result
 */
function calculate(a, b) {
    const sum = (0, utils_1.add)(a, b); // Outgoing call to add
    let result = (0, utils_1.multiply)(sum, 2); // Outgoing call to multiply
    // Additional logic for testing
    if (result > 50) {
        result = (0, utils_1.subtract)(result, 10); // Outgoing call to subtract
    }
    return result;
}
/**
 * Advanced calculation that also uses the calculate function
 * This creates another pathway in the call hierarchy
 * @param values Array of values to process
 * @returns Advanced result
 */
function advancedCalculation(values) {
    let total = 0;
    for (const value of values) {
        total += calculate(value, 1); // Incoming call to calculate
    }
    return total;
}
/**
 * Complex business logic class
 */
class BusinessLogic {
    constructor(multiplier) {
        this.multiplier = multiplier;
    }
    /**
     * Process a value using internal logic
     * @param value Input value
     * @returns Processed result
     */
    processValue(value) {
        return calculate(value, this.multiplier); // Another incoming call to calculate
    }
    /**
     * Complex processing that chains multiple calls
     * @param data Array of input data
     * @returns Processed results
     */
    processArray(data) {
        return data.map(item => {
            const intermediate = (0, utils_1.add)(item, 5); // Outgoing call to add
            return calculate(intermediate, 2); // Outgoing call to calculate
        });
    }
}
exports.BusinessLogic = BusinessLogic;
//# sourceMappingURL=calculator.js.map
// Calculator module with core business logic

const { add, multiply, subtract } = require('./utils');

/**
 * Calculate performs a complex calculation using utility functions
 * This function should show both incoming calls (from main, processNumbers, Calculator)
 * and outgoing calls (to add, multiply, subtract)
 * @param {number} a First operand
 * @param {number} b Second operand
 * @returns {number} Calculated result
 */
function calculate(a, b) {
    const sum = add(a, b);        // Outgoing call to add
    let result = multiply(sum, 2); // Outgoing call to multiply
    
    // Additional logic for testing
    if (result > 50) {
        result = subtract(result, 10); // Outgoing call to subtract
    }
    
    return result;
}

/**
 * Advanced calculation that also uses the calculate function
 * This creates another pathway in the call hierarchy
 * @param {number[]} values Array of values to process
 * @returns {number} Advanced result
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
    /**
     * @param {number} multiplier The multiplier value
     */
    constructor(multiplier) {
        this.multiplier = multiplier;
    }
    
    /**
     * Process a value using internal logic
     * @param {number} value Input value
     * @returns {number} Processed result
     */
    processValue(value) {
        return calculate(value, this.multiplier); // Another incoming call to calculate
    }
    
    /**
     * Complex processing that chains multiple calls
     * @param {number[]} data Array of input data
     * @returns {number[]} Processed results
     */
    processArray(data) {
        return data.map(item => {
            const intermediate = add(item, 5); // Outgoing call to add
            return calculate(intermediate, 2); // Outgoing call to calculate
        });
    }
    
    /**
     * Static factory method
     * @param {number} multiplier Initial multiplier
     * @returns {BusinessLogic} New instance
     */
    static create(multiplier) {
        return new BusinessLogic(multiplier);
    }
}

/**
 * Functional approach to business logic
 * @param {number} multiplier The multiplier to use
 * @returns {Function} A function that processes values
 */
function createProcessor(multiplier) {
    return function(value) {
        return calculate(value, multiplier); // Incoming call to calculate
    };
}

// Export all functions and classes
module.exports = {
    calculate,
    advancedCalculation,
    BusinessLogic,
    createProcessor
};
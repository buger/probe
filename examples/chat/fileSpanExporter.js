import { createWriteStream } from 'fs';
import corePkg from '@opentelemetry/core';

const { ExportResultCode } = corePkg;

/**
 * File exporter for OpenTelemetry spans
 * Exports spans to a file in JSON Lines format (one JSON object per line)
 * Following the OTLP JSON format specification
 */
export class FileSpanExporter {
  constructor(filePath = './traces.jsonl') {
    this.filePath = filePath;
    this.stream = createWriteStream(filePath, { flags: 'a' });
    this.stream.on('error', (error) => {
      console.error(`[FileSpanExporter] Stream error: ${error.message}`);
    });
  }

  /**
   * Export spans to file
   * @param {ReadableSpan[]} spans - Array of spans to export
   * @param {function} resultCallback - Callback to call with the export result
   */
  export(spans, resultCallback) {
    if (!spans || spans.length === 0) {
      resultCallback({ code: ExportResultCode.SUCCESS });
      return;
    }

    try {
      const timestamp = Date.now();
      
      spans.forEach((span) => {
        // Convert span to OTLP JSON format
        const spanData = {
          traceId: span.spanContext().traceId,
          spanId: span.spanContext().spanId,
          parentSpanId: span.parentSpanId,
          name: span.name,
          kind: span.kind,
          startTimeUnixNano: span.startTime[0] * 1_000_000_000 + span.startTime[1],
          endTimeUnixNano: span.endTime[0] * 1_000_000_000 + span.endTime[1],
          attributes: this.convertAttributes(span.attributes),
          status: span.status,
          events: span.events?.map(event => ({
            timeUnixNano: event.time[0] * 1_000_000_000 + event.time[1],
            name: event.name,
            attributes: this.convertAttributes(event.attributes),
          })) || [],
          links: span.links?.map(link => ({
            traceId: link.context.traceId,
            spanId: link.context.spanId,
            attributes: this.convertAttributes(link.attributes),
          })) || [],
          resource: {
            attributes: this.convertAttributes(span.resource?.attributes || {}),
          },
          instrumentationLibrary: {
            name: span.instrumentationLibrary?.name || 'unknown',
            version: span.instrumentationLibrary?.version || 'unknown',
          },
          timestamp,
        };

        // Write as JSON Lines format (one JSON object per line)
        this.stream.write(JSON.stringify(spanData) + '\n');
      });

      resultCallback({ code: ExportResultCode.SUCCESS });
    } catch (error) {
      console.error(`[FileSpanExporter] Export error: ${error.message}`);
      resultCallback({ 
        code: ExportResultCode.FAILED, 
        error: error 
      });
    }
  }

  /**
   * Convert OpenTelemetry attributes to plain object
   * @param {Object} attributes - OpenTelemetry attributes
   * @returns {Object} Plain object with string values
   */
  convertAttributes(attributes) {
    if (!attributes) return {};
    
    const result = {};
    for (const [key, value] of Object.entries(attributes)) {
      // Convert all values to strings for JSON compatibility
      if (typeof value === 'object' && value !== null) {
        result[key] = JSON.stringify(value);
      } else {
        result[key] = String(value);
      }
    }
    return result;
  }

  /**
   * Shutdown the exporter
   * @returns {Promise<void>}
   */
  async shutdown() {
    return new Promise((resolve) => {
      if (this.stream) {
        this.stream.end(() => {
          console.log(`[FileSpanExporter] File stream closed: ${this.filePath}`);
          resolve();
        });
      } else {
        resolve();
      }
    });
  }

  /**
   * Force flush any pending spans
   * @returns {Promise<void>}
   */
  async forceFlush() {
    return new Promise((resolve) => {
      if (this.stream) {
        this.stream.once('drain', resolve);
        // If stream is not busy, resolve immediately
        if (!this.stream.writableNeedDrain) {
          resolve();
        }
      } else {
        resolve();
      }
    });
  }
}
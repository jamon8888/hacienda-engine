export interface PIIEntity {
  type: string;
  value: string;
  start: number;
  end: number;
  confidence: number;
}

export const PII_PATTERNS: Record<string, RegExp> = {
  email: /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b/g,
  phone: /(\+?\d{1,3}[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}/g,
  ssn: /\b\d{3}-\d{2}-\d{4}\b/g,
  creditCard: /\b(?:\d{4}[-\s]?){3}\d{4}\b/g,
  iban: /\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b/g,
  bic: /\b[A-Z]{6}[A-Z0-9]{2}(?:[A-Z0-9]{3})?\b/g,
  passport: /\b[A-Z]{1,2}\d{6,9}\b/g,
  driversLicense: /\b[A-Z]\d{7,13}\b/g,
  euVat:
    /\b(?:AT|BE|BG|CY|CZ|DE|DK|EE|EL|ES|FI|FR|HR|HU|IE|IT|LT|LU|LV|MT|NL|PL|PT|RO|SE|SI|SK)\d{2,12}\b/gi,
};

export function detectPII(text: string): PIIEntity[] {
  const entities: PIIEntity[] = [];
  for (const [type, pattern] of Object.entries(PII_PATTERNS)) {
    let match;
    while ((match = pattern.exec(text)) !== null) {
      entities.push({
        type,
        value: match[0],
        start: match.index,
        end: match.index + match[0].length,
        confidence: 0.95,
      });
    }
  }
  return entities;
}

export function redactPII(text: string, entities: PIIEntity[]): string {
  let result = text;
  const sorted = [...entities].sort((a, b) => b.start - a.start);
  for (const entity of sorted) {
    const replacement = `[${entity.type.toUpperCase()}]`;
    result =
      result.slice(0, entity.start) + replacement + result.slice(entity.end);
  }
  return result;
}

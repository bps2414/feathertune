import { pickLocale } from './index';

describe('pickLocale', () => {
  it('uses pt-BR for Portuguese and English for everything else', () => {
    expect(pickLocale('pt')).toBe('pt-BR');
    expect(pickLocale('PT')).toBe('pt-BR');
    expect(pickLocale('en')).toBe('en');
    expect(pickLocale('de')).toBe('en');
    expect(pickLocale(null)).toBe('en');
    expect(pickLocale(undefined)).toBe('en');
  });
});

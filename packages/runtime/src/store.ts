export interface PrivateStore {
  list(prefix: string): Promise<string[]>;
  read(id: string): Promise<unknown | undefined>;
  create(id: string, value: unknown): Promise<boolean>;
}

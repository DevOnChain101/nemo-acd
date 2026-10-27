export function connect() {
  const url = process.env.DB_URL;
  if (!url) throw new Error('DB_URL is not set');
  return { url };
}

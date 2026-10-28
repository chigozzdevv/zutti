export function Token({ asset, small = false }: { asset: "ZEC" | "USDC"; small?: boolean }) {
  return (
    <span
      className={`token token-${asset.toLowerCase()}${small ? " token-small" : ""}`}
      aria-hidden="true"
    >
      {asset === "ZEC" ? (
        <svg viewBox="0 0 24 24">
          <path d="M7 6h10L7 18h10M12 3v3m0 12v3" />
        </svg>
      ) : (
        <svg viewBox="0 0 24 24">
          <path d="M14.8 8.2c-.8-.7-1.7-1-2.8-1-1.8 0-2.9.8-2.9 2s1.3 1.7 3 2c1.8.4 3 .9 3 2.4s-1.3 2.4-3.1 2.4c-1.2 0-2.3-.5-3.1-1.2M12 5.4v1.8m0 8.8v2.2M5.8 5.8a9 9 0 0 0 0 12.4M18.2 5.8a9 9 0 0 1 0 12.4" />
        </svg>
      )}
    </span>
  );
}

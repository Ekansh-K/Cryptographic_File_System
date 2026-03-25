interface Props {
  data: Uint8Array;
}

const MAX_HEX_BYTES = 4096;

export default function HexViewer({ data }: Props) {
  const displayData = data.length > MAX_HEX_BYTES ? data.slice(0, MAX_HEX_BYTES) : data;
  const rows: { offset: number; hex: string[]; ascii: string }[] = [];

  for (let i = 0; i < displayData.length; i += 16) {
    const chunk = displayData.slice(i, i + 16);
    const hex: string[] = [];
    let ascii = "";
    for (let j = 0; j < 16; j++) {
      if (j < chunk.length) {
        hex.push(chunk[j].toString(16).padStart(2, "0"));
        ascii += chunk[j] >= 0x20 && chunk[j] < 0x7f ? String.fromCharCode(chunk[j]) : ".";
      } else {
        hex.push("  ");
        ascii += " ";
      }
    }
    rows.push({ offset: i, hex, ascii });
  }

  return (
    <div className="font-mono text-sm">
      {data.length > MAX_HEX_BYTES && (
        <div className="px-2 py-1 text-xs text-text-muted border-b border-border">
          Showing first 4 KB of hex data
        </div>
      )}
      <table className="w-full">
        <tbody>
          {rows.map((row) => (
            <tr key={row.offset} className="hover:bg-surface-hover">
              <td className="text-text-muted select-none px-2 w-16 text-right border-r border-border">
                {row.offset.toString(16).padStart(8, "0")}
              </td>
              <td className="px-2 border-r border-border">
                <span className="text-text">
                  {row.hex.slice(0, 8).join(" ")}
                </span>
                <span className="text-text-muted mx-1">│</span>
                <span className="text-text">
                  {row.hex.slice(8).join(" ")}
                </span>
              </td>
              <td className="px-2 text-text-muted whitespace-pre">
                {row.ascii}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

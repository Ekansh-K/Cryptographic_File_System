interface Props {
  data: Uint8Array;
}

export default function TextViewer({ data }: Props) {
  const text = new TextDecoder("utf-8", { fatal: false }).decode(data);
  const lines = text.split("\n");

  return (
    <div className="font-mono text-sm">
      <table className="w-full">
        <tbody>
          {lines.map((line, i) => (
            <tr key={i} className="hover:bg-surface-hover">
              <td className="text-right text-text-muted select-none px-2 w-10 align-top border-r border-border">
                {i + 1}
              </td>
              <td className="px-2 whitespace-pre-wrap break-all text-text">
                {line}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

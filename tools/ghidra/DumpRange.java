// Disassemble an address range into text without saving the program.
// @category CODOL
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Instruction;
import java.io.PrintWriter;

public class DumpRange extends GhidraScript {
    public void run() throws Exception {
        String[] a = getScriptArgs();
        try (PrintWriter out = new PrintWriter(a[0], "UTF-8")) {
            for (int r = 1; r + 1 < a.length; r += 2) {
                Address cur = toAddr(Long.parseUnsignedLong(a[r].replace("0x", ""), 16));
                Address end = toAddr(Long.parseUnsignedLong(a[r + 1].replace("0x", ""), 16));
                out.println("; range " + cur + " - " + end);
                while (cur.compareTo(end) < 0) {
                    Instruction ins = getInstructionAt(cur);
                    if (ins == null) { disassemble(cur); ins = getInstructionAt(cur); }
                    if (ins == null) { out.println(cur + "  db " + String.format("%02x", getByte(cur) & 0xff)); cur = cur.add(1); continue; }
                    out.println(cur + "  " + ins);
                    cur = ins.getMaxAddress().add(1);
                }
            }
        }
    }
}

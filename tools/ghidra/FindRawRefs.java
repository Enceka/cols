// Scan loaded memory for absolute pointers and E8/E9 rel32 branches to targets.
// @category CODOL
import ghidra.app.script.GhidraScript;
import ghidra.program.model.mem.MemoryBlock;
import java.io.PrintWriter;

public class FindRawRefs extends GhidraScript {
    public void run() throws Exception {
        String[] args = getScriptArgs();
        try (PrintWriter output = new PrintWriter(args[0], "UTF-8")) {
            for (MemoryBlock block : currentProgram.getMemory().getBlocks()) {
                if (!block.isInitialized()) continue;
                byte[] data = new byte[(int) block.getSize()];
                block.getBytes(block.getStart(), data);
                long start = block.getStart().getOffset();
                for (int t = 1; t < args.length; t++) {
                    long target = Long.parseUnsignedLong(args[t].replace("0x", ""), 16);
                    for (int i = 0; i + 4 <= data.length; i++) {
                        long v = (data[i] & 0xffL) | (data[i+1] & 0xffL) << 8 | (data[i+2] & 0xffL) << 16 | (data[i+3] & 0xffL) << 24;
                        if (v == target) {
                            String kind = "ptr";
                            if (i > 0 && (data[i-1] & 0xff) == 0x68) kind = "push";
                            output.println(String.format("%08x %s %s %s", start + i, kind, block.getName(), args[t]));
                        }
                        if (i > 0 && ((data[i-1] & 0xff) == 0xe8 || (data[i-1] & 0xff) == 0xe9)) {
                            int rel = (int) v;
                            if (start + i + 4 + rel == target)
                                output.println(String.format("%08x %s %s %s", start + i - 1, (data[i-1] & 0xff) == 0xe8 ? "call" : "jmp", block.getName(), args[t]));
                        }
                    }
                }
            }
        }
    }
}

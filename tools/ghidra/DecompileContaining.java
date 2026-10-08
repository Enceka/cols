// Export the functions containing selected VAs; never execute the program.
// @category CODOL
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import java.io.PrintWriter;

public class DecompileContaining extends GhidraScript {
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 2) throw new IllegalArgumentException("output path followed by target VAs");
        DecompInterface decompiler = new DecompInterface();
        decompiler.openProgram(currentProgram);
        try (PrintWriter output = new PrintWriter(args[0], "UTF-8")) {
            output.println("// Program: " + currentProgram.getName());
            output.println("// SHA256: " + currentProgram.getExecutableSHA256());
            for (int i = 1; i < args.length; i++) {
                Address address = toAddr(Long.parseUnsignedLong(args[i].replace("0x", ""), 16));
                Function function = getFunctionContaining(address);
                if (function == null) {
                    disassemble(address);
                    function = createFunction(address, "research_" + address.toString());
                }
                output.println("\n// TARGET VA " + address);
                if (function == null) { output.println("// No function created"); continue; }
                DecompileResults result = decompiler.decompileFunction(function, 45, monitor);
                if (result.decompileCompleted()) output.println(result.getDecompiledFunction().getC());
                else output.println("// ERROR: " + result.getErrorMessage());
                output.flush();
            }
        } finally { decompiler.dispose(); }
    }
}

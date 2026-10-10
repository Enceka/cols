// Export functions that reference selected VAs; never execute the program.
// @category CODOL
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.symbol.Reference;
import java.io.PrintWriter;
import java.util.LinkedHashSet;

public class DecompileCallers extends GhidraScript {
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 2) throw new IllegalArgumentException("output path followed by target VAs");
        DecompInterface decompiler = new DecompInterface();
        decompiler.openProgram(currentProgram);
        try (PrintWriter output = new PrintWriter(args[0], "UTF-8")) {
            output.println("// Program: " + currentProgram.getName());
            LinkedHashSet<Function> callers = new LinkedHashSet<>();
            for (int i = 1; i < args.length; i++) {
                Address target = toAddr(Long.parseUnsignedLong(args[i].replace("0x", ""), 16));
                for (Reference ref : getReferencesTo(target)) {
                    Function f = getFunctionContaining(ref.getFromAddress());
                    output.println("// REF " + target + " <- " + ref.getFromAddress() + " " + ref.getReferenceType()
                        + " in " + (f == null ? "<none>" : f.getEntryPoint().toString()));
                    if (f != null) callers.add(f);
                }
            }
            for (Function f : callers) {
                output.println("\n// CALLER VA " + f.getEntryPoint());
                DecompileResults result = decompiler.decompileFunction(f, 60, monitor);
                if (result.decompileCompleted()) output.println(result.getDecompiledFunction().getC());
                else output.println("// ERROR: " + result.getErrorMessage());
            }
        } finally { decompiler.dispose(); }
    }
}

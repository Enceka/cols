// Read a bounded static word table from the analysis image; never execute it.
// @category CODOL
import ghidra.app.script.GhidraScript;
import java.io.PrintWriter;
public class DumpWords extends GhidraScript {
    public void run() throws Exception {
        String[] args=getScriptArgs();
        try(PrintWriter out=new PrintWriter(args[0],"UTF-8")) {
            for(int i=1;i<args.length;i++) {
                long address=Long.parseUnsignedLong(args[i].replace("0x",""),16);
                out.printf("%08x %08x%n",address,getInt(toAddr(address)));
            }
        }
    }
}

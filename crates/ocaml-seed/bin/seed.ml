(* moof-seed CLI entry point.

   for the project-skeleton phase, this just reads stdin (or a file
   given as the first argv) and prints the parsed form(s). subsequent
   agents extend this with `compile` / `build-image` subcommands. *)

let () =
  let text =
    if Array.length Sys.argv > 1 then
      let ic = open_in Sys.argv.(1) in
      let n = in_channel_length ic in
      let buf = Bytes.create n in
      really_input ic buf 0 n;
      close_in ic;
      Bytes.to_string buf
    else
      let buf = Buffer.create 1024 in
      (try
         while true do
           Buffer.add_channel buf stdin 4096
         done
       with End_of_file -> ());
      Buffer.contents buf
  in
  try
    let forms = Moof_seed.Reader.read_all text in
    List.iter (fun f -> print_endline (Moof_seed.Ast.to_string f)) forms
  with Moof_seed.Reader.ReadError msg ->
    prerr_endline msg;
    exit 1

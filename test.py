import socket

def connect():
    tcp_socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server_addr = ("127.0.0.1", 5555)
    tcp_socket.connect(server_addr)

    send_data = "connect 123"
    tcp_socket.send(send_data.encode("utf8"))
    recv_data = tcp_socket.recv(1024)
    tcp_socket.settimeout(50)
    print('recv connect result:', recv_data.decode("utf8"))
    
connect()